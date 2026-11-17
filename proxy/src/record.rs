//! Writing the record (ADR-057): one line for each request and each site asked for, in the person's folder,
//! in files by day and part, kept within a bound. The words of a line are `torq_wire::activity`'s.
//!
//! The folders are made for the proxy (`modules/model-proxy.nix`): the proxy's own, with an access rule that
//! lets the one person read. The proxy makes no folder itself. A file is made readable by the proxy's group
//! (0640) because that is what an access rule's limit is taken from; no one is in that group.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Mutex;
use torq_wire::activity::{Entry, Line, Lost, Removed, MAX_FILE, MAX_PERSON, VERSION};

/// The bounds of one person's record.
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    /// How large a file grows before the next part is begun.
    pub file: u64,
    /// How much is kept for a person: past it the oldest files go.
    pub person: u64,
}

impl Default for Bounds {
    fn default() -> Self {
        Bounds { file: MAX_FILE, person: MAX_PERSON }
    }
}

/// What is known of one person's folder between writes.
#[derive(Debug, Default)]
struct Folder {
    /// The file being written and its size; none until the first write looks.
    current: Option<(String, u64)>,
    /// Every file's size together.
    total: u64,
    /// Lines that could not be written since the last one that could.
    lost: u64,
}

impl Folder {
    /// So many bytes more in the file being written.
    fn wrote(&mut self, bytes: u64) {
        self.total += bytes;
        if let Some(c) = self.current.as_mut() {
            c.1 += bytes;
        }
    }
}

/// The record, for every person.
pub struct Recorder {
    dir: PathBuf,
    bounds: Bounds,
    folders: Mutex<HashMap<String, Folder>>,
}

/// A file of the record by its name: `<day>.<part>.jsonl`, the part written as the proxy writes it (`part_name`):
/// three digits at least, more past 999 (`1000`), never a zero in front of a longer one. Its day and part, or
/// not one of the record's.
fn part_of(name: &str) -> Option<(&str, u32)> {
    let (day, rest) = name.strip_suffix(".jsonl")?.split_once('.')?;
    let dated = day.len() == 10 && day.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() });
    if !dated || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u32 = rest.parse().ok()?;
    (rest == format!("{n:03}")).then_some((day, n))
}

/// The name of part `n` of `day`'s file.
fn part_name(day: &str, n: u32) -> String {
    format!("{day}.{n:03}.jsonl")
}

impl Recorder {
    pub fn new(dir: PathBuf, bounds: Bounds) -> Self {
        Recorder { dir, bounds, folders: Mutex::new(HashMap::new()) }
    }

    /// The record's files in a person's folder, oldest first (by day, then by part as a number: `1000` comes
    /// after `999`, though not by name), each with its size.
    fn files(&self, person: &str) -> io::Result<Vec<(String, u64)>> {
        let mut files = Vec::new();
        for e in fs::read_dir(self.dir.join(person))? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some((day, n)) = part_of(&name) {
                files.push(((day.to_string(), n), name.clone(), e.metadata()?.len()));
            }
        }
        files.sort();
        Ok(files.into_iter().map(|(_, name, len)| (name, len)).collect())
    }

    /// One line appended, as a single write: a reader never sees half of one.
    fn append(&self, person: &str, file: &str, line: &Line) -> io::Result<u64> {
        let mut text = serde_json::to_vec(&Entry { v: VERSION, line: line.clone() }).map_err(io::Error::other)?;
        text.push(b'\n');
        let mut f = fs::OpenOptions::new().append(true).create(true).mode(0o640).open(self.dir.join(person).join(file))?;
        f.write_all(&text)?;
        Ok(text.len() as u64)
    }

    /// Where a line of about `size` bytes goes for `person` at `now` (seconds): the day's file, its next part
    /// where it is full, and first the oldest files removed where the person's bound would be passed, each
    /// removal a line of its own.
    fn place(&self, folder: &mut Folder, person: &str, size: u64, now: u64) -> io::Result<String> {
        let day = crate::time::day(now);
        if folder.current.is_none() {
            let files = self.files(person)?;
            folder.total = files.iter().map(|(_, n)| n).sum();
            folder.current = files.last().cloned();
        }
        let next = match &folder.current {
            Some((name, len)) => match part_of(name) {
                Some((d, _)) if d == day && len + size <= self.bounds.file => None,
                Some((d, n)) if d == day => Some(part_name(&day, n.saturating_add(1))),
                _ => Some(part_name(&day, 0)),
            },
            None => Some(part_name(&day, 0)),
        };
        if let Some(name) = next {
            folder.current = Some((name, 0));
        }
        let current = folder.current.as_ref().map(|(n, _)| n.clone()).unwrap_or_default();
        if folder.total + size > self.bounds.person {
            for (name, len) in self.files(person)? {
                if folder.total + size <= self.bounds.person || name == current {
                    break;
                }
                fs::remove_file(self.dir.join(person).join(&name))?;
                folder.total = folder.total.saturating_sub(len);
                let told = self.append(person, &current, &Line::Removed(Removed { time: crate::time::rfc3339(now), file: name }))?;
                folder.wrote(told);
            }
        }
        Ok(current)
    }

    /// `line` written to `person`'s record at `now`.
    fn put(&self, folder: &mut Folder, person: &str, line: &Line, now: u64) -> io::Result<()> {
        // About what the line takes: enough to decide where it goes.
        let size = serde_json::to_vec(line).map(|v| v.len() as u64 + 16).unwrap_or(0);
        let current = self.place(folder, person, size, now)?;
        let wrote = self.append(person, &current, line)?;
        folder.wrote(wrote);
        Ok(())
    }

    /// `line` written to `person`'s record. One that cannot be written is counted, and the next thing asked
    /// for that person is refused until the record can be written again (`ready`).
    pub fn write(&self, person: &str, line: Line) {
        let mut folders = self.folders.lock().unwrap_or_else(|e| e.into_inner());
        let folder = folders.entry(person.to_string()).or_default();
        if let Err(e) = self.put(folder, person, &line, crate::time::now()) {
            folder.lost += 1;
            folder.current = None;
            eprintln!("torq-model-proxy: {person}'s record could not be written ({e}): nothing more is passed on for their agent until it can be");
        }
    }

    /// Whether `person`'s record can be written: asked before anything is passed on for their agent. After a
    /// failure, it is tried again here, with a line that says how many were lost.
    pub fn ready(&self, person: &str) -> bool {
        let mut folders = self.folders.lock().unwrap_or_else(|e| e.into_inner());
        let folder = folders.entry(person.to_string()).or_default();
        let now = crate::time::now();
        if folder.lost == 0 {
            // Looked at before the first thing is passed on, and again after any failure: the folder is
            // there and a file in it can be opened to write.
            if folder.current.is_some() {
                return true;
            }
            let opened = self.place(folder, person, 0, now).and_then(|file| fs::OpenOptions::new().append(true).create(true).mode(0o640).open(self.dir.join(person).join(file)));
            if opened.is_err() {
                folder.current = None;
            }
            return opened.is_ok();
        }
        let line = Line::Lost(Lost { time: crate::time::rfc3339(now), lines: folder.lost });
        match self.put(folder, person, &line, now) {
            Ok(()) => {
                folder.lost = 0;
                true
            }
            Err(_) => {
                folder.current = None;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use torq_wire::activity::{Connect, Why};

    fn line(host: &str) -> Line {
        Line::Connect(Connect { time: "t".into(), agent: "torq-agent-alice".into(), person: "alice".into(), host: host.into(), port: 443, allowed: false, why: Why::NotListed, reached: None })
    }

    fn read(dir: &std::path::Path) -> Vec<(String, Vec<Entry>)> {
        let mut names: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".jsonl")).collect();
        names.sort();
        names.into_iter().map(|n| {
            let text = fs::read_to_string(dir.join(&n)).unwrap();
            (n, text.lines().map(|l| serde_json::from_str(l).unwrap()).collect())
        }).collect()
    }

    #[test]
    fn a_line_goes_in_the_days_file_and_the_next_day_in_its_own() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("alice")).unwrap();
        let r = Recorder::new(d.path().into(), Bounds::default());
        let mut f = Folder::default();
        // 2026-11-16T08:59:59Z, and a day later.
        r.put(&mut f, "alice", &line("a.example"), 1_794_819_599).unwrap();
        r.put(&mut f, "alice", &line("b.example"), 1_794_819_600).unwrap();
        r.put(&mut f, "alice", &line("c.example"), 1_794_819_599 + 86_400).unwrap();
        let files = read(&d.path().join("alice"));
        assert_eq!(files.iter().map(|(n, e)| (n.as_str(), e.len())).collect::<Vec<_>>(), [("2026-11-16.000.jsonl", 2), ("2026-11-17.000.jsonl", 1)]);
        assert_eq!(files[0].1[1].line, line("b.example"));
        let mode = std::os::unix::fs::PermissionsExt::mode(&fs::metadata(d.path().join("alice/2026-11-16.000.jsonl")).unwrap().permissions()) & 0o777;
        assert_eq!(mode & 0o137, 0, "never the world's, never written by the group: {mode:o}");
    }

    #[test]
    fn a_full_file_is_followed_by_its_next_part_and_a_new_proxy_goes_on_where_the_last_stopped() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("alice")).unwrap();
        let bounds = Bounds { file: 400, person: 1_000_000 };
        let r = Recorder::new(d.path().into(), bounds);
        let mut f = Folder::default();
        for i in 0..6 {
            r.put(&mut f, "alice", &line(&format!("{i}.example")), 1_794_819_599).unwrap();
        }
        let files = read(&d.path().join("alice"));
        assert!(files.len() >= 3, "several parts: {}", files.len());
        assert_eq!(files[1].0, "2026-11-16.001.jsonl");
        assert!(files.iter().all(|(n, _)| fs::metadata(d.path().join("alice").join(n)).unwrap().len() <= 400), "none past its bound");
        assert_eq!(files.iter().map(|(_, e)| e.len()).sum::<usize>(), 6, "every line, once");
        // Started again: it finds the last part and goes on in it or after it, never over an older one.
        let again = Recorder::new(d.path().into(), bounds);
        let mut f = Folder::default();
        again.put(&mut f, "alice", &line("later.example"), 1_794_819_599).unwrap();
        let after = read(&d.path().join("alice"));
        assert_eq!(after.iter().map(|(_, e)| e.len()).sum::<usize>(), 7);
        assert_eq!(after.last().unwrap().1.last().unwrap().line, line("later.example"));
    }

    #[test]
    fn past_a_persons_bound_the_oldest_files_go_and_the_record_says_which() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("alice")).unwrap();
        fs::write(d.path().join("alice/notes.txt"), "not the record's").unwrap();
        let r = Recorder::new(d.path().into(), Bounds { file: 400, person: 1500 });
        let mut f = Folder::default();
        for i in 0..40 {
            r.put(&mut f, "alice", &line(&format!("{i}.example")), 1_794_819_599 + i * 3600).unwrap();
        }
        let files = read(&d.path().join("alice"));
        let total: u64 = files.iter().map(|(n, _)| fs::metadata(d.path().join("alice").join(n)).unwrap().len()).sum();
        assert!(total <= 1500, "kept within the bound: {total}");
        let all: Vec<&Entry> = files.iter().flat_map(|(_, e)| e).collect();
        assert_eq!(all.last().unwrap().line, line("39.example"), "the newest is kept");
        assert!(!all.iter().any(|e| e.line == line("0.example")), "the oldest is gone");
        assert!(all.iter().any(|e| matches!(&e.line, Line::Removed(r) if r.file.ends_with(".jsonl"))), "and a line says a file was removed");
        assert!(d.path().join("alice/notes.txt").exists(), "a file that is not the record's is not touched");
    }

    /// Past part 999 of a day the parts go on (1000, 1001, …), in order by their number, not by their name: none
    /// wraps to 000 or is left out of the count, and the oldest go first, never the newest (the security reader,
    /// 2026-11-17).
    #[test]
    fn past_part_999_of_a_day_the_parts_go_on_in_order() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("alice")).unwrap();
        // One line a file; room for 400 files' worth, so the oldest go once the parts pass 999.
        let one = serde_json::to_vec(&Entry { v: VERSION, line: line("0000.example") }).unwrap().len() as u64 + 1;
        let r = Recorder::new(d.path().into(), Bounds { file: 1, person: 400 * one + 2_000 });
        let mut f = Folder::default();
        for i in 0..1_010 {
            r.put(&mut f, "alice", &line(&format!("{i:04}.example")), 1_794_819_599).unwrap();
        }
        let names = r.files("alice").unwrap();
        let last = &names.last().unwrap().0;
        assert_eq!(last, "2026-11-16.1009.jsonl", "the newest part is the last, by its number ({} files)", names.len());
        let text = fs::read_to_string(d.path().join("alice").join(last)).unwrap();
        assert!(text.contains("1009.example"), "the newest line is kept, in the newest part: {text}");
        assert!(!d.path().join("alice/2026-11-16.000.jsonl").exists(), "the oldest went: 000 did not come back");
        let total: u64 = fs::read_dir(d.path().join("alice")).unwrap().map(|e| e.unwrap().metadata().unwrap().len()).sum();
        assert!(total <= 400 * one + 2_000, "every file counted, within the bound: {total}");
        // Started again: it goes on after 1009, never over an older part.
        let again = Recorder::new(d.path().into(), Bounds { file: 1, person: 400 * one + 2_000 });
        again.put(&mut Folder::default(), "alice", &line("later.example"), 1_794_819_599).unwrap();
        assert_eq!(again.files("alice").unwrap().last().unwrap().0, "2026-11-16.1010.jsonl");
    }

    /// A line's size is counted as it is written, escaped: a text of control characters is six times as long in
    /// JSON, and a file still never passes its bound by more than the one line that begins it.
    #[test]
    fn a_lines_size_is_counted_as_written_escaped() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("alice")).unwrap();
        let r = Recorder::new(d.path().into(), Bounds { file: 4_000, person: 1_000_000 });
        let mut f = Folder::default();
        let escaped = "\u{1}".repeat(100);
        for _ in 0..40 {
            r.put(&mut f, "alice", &line(&escaped), 1_794_819_599).unwrap();
        }
        for (name, len) in r.files("alice").unwrap() {
            assert!(len <= 4_000, "{name}: {len}");
        }
    }

    #[test]
    fn a_record_that_cannot_be_written_stops_the_agent_until_it_can_and_says_what_was_lost() {
        let d = tempfile::tempdir().unwrap();
        let r = Recorder::new(d.path().into(), Bounds::default());
        fs::create_dir(d.path().join("bob")).unwrap();
        // No folder for alice: nothing can be written, and that is known before anything is passed on.
        assert!(!r.ready("alice"), "refused from the first");
        r.write("alice", line("a.example"));
        r.write("alice", line("b.example"));
        assert!(!r.ready("alice"), "refused while it cannot be written");
        assert!(r.ready("bob"), "another person's is their own");
        fs::create_dir(d.path().join("alice")).unwrap();
        assert!(r.ready("alice"), "and passed on again once it can");
        let files = read(&d.path().join("alice"));
        assert_eq!(files[0].1[0].line, Line::Lost(Lost { time: files[0].1[0].line.clone().time(), lines: 2 }));
        assert!(r.ready("alice"));
        assert_eq!(read(&d.path().join("alice"))[0].1.len(), 1, "said once");
    }

    #[test]
    fn only_the_records_own_names_are_its_files() {
        assert_eq!(part_of("2026-11-16.000.jsonl"), Some(("2026-11-16", 0)));
        assert_eq!(part_of("2026-11-16.012.jsonl"), Some(("2026-11-16", 12)));
        assert_eq!(part_of("2026-11-16.1000.jsonl"), Some(("2026-11-16", 1000)));
        assert_eq!(part_of("2026-11-16.123456.jsonl"), Some(("2026-11-16", 123_456)));
        for not in ["2026-11-16.jsonl", "2026-11-16.0.jsonl", "2026-11-16.0001.jsonl", "2026-11-16.99999999999.jsonl", "2026-11-16.+12.jsonl", "notes.txt", "2026-11-16.000.json", "x026-11-16.000.jsonl", "../2026-11-16.000.jsonl"] {
            assert_eq!(part_of(not), None, "{not}");
        }
    }

    trait Time {
        fn time(self) -> String;
    }
    impl Time for Line {
        fn time(self) -> String {
            match self {
                Line::Lost(l) => l.time,
                _ => String::new(),
            }
        }
    }
}
