//! A person's standing instructions for their agent (ADR-055; the owner, 2026-11-15): a text of their own
//! that their agent is always given — "Always answer in German", "I am new to Linux: explain commands".
//!
//! The text is the person's, in the person's home: `~/.config/torq/agent-instructions.md`. Their agent cannot
//! write there (it has no way into the home, and a hidden folder of the home is never shared with it), so
//! nothing the agent read — a web page, a file — can rewrite what it is instructed. The person's own programs
//! read it: `torq ask`, the agent's window and applet send it with each question (`client::Client`), and
//! `claude` typed in a terminal gives it to Claude Code as it starts it. At most 16 KiB of text; a file that is
//! larger, not text, or a link is not used, and that is said.

use std::io::Read;
use std::path::{Path, PathBuf};
use torq_wire::agent::{instructions_ok, MAX_INSTRUCTIONS};

/// Where the instructions are, inside the person's home.
pub const FILE: &str = ".config/torq/agent-instructions.md";

/// What the Agent page and `torq agent instructions` say they are for.
pub const WHAT_FOR: &str = "What you write here, your agent is always told: in its window, in the panel, with `torq ask`, and when you type `claude`. For example \"Always answer in German\" or \"I am new to Linux: explain commands\". Only you can change it: your agent cannot.";

/// The person's instructions file: in their home, as the environment names it.
pub fn path() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(|h| Path::new(&h).join(FILE))
}

/// The person's instructions, as they stand.
#[derive(Debug, Clone, PartialEq)]
pub enum Instructions {
    /// None written (no file, or an empty one).
    None,
    /// The text.
    Text(String),
    /// There is a file, and it cannot be used: why, as the person is told. Their agent works without it.
    Unusable(String),
}

impl Instructions {
    /// The text, if there is one to give the agent.
    pub fn text(&self) -> Option<&str> {
        match self {
            Instructions::Text(t) => Some(t),
            _ => None,
        }
    }

    /// What the person is told when they cannot be used.
    pub fn unusable(&self) -> Option<&str> {
        match self {
            Instructions::Unusable(why) => Some(why),
            _ => None,
        }
    }
}

/// The instructions in `file`: read without following a link and without waiting, a regular file only, and
/// no further than the bound.
pub fn read(file: &Path) -> Instructions {
    use rustix::fs::OFlags;
    use std::os::unix::fs::OpenOptionsExt;
    let unusable = |why: &str| Instructions::Unusable(format!("Your instructions for your agent are not used: {why} ({}). Your agent works without them until that is put right: `torq agent instructions edit` opens them, `torq agent instructions clear` removes them.", file.display()));
    let f = match std::fs::OpenOptions::new().read(true).custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK).bits() as i32).open(file) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Instructions::None,
        Err(e) if rustix::io::Errno::from_io_error(&e) == Some(rustix::io::Errno::LOOP) => return unusable("the file is a link to another file, and a link is not followed"),
        Err(e) => return unusable(&format!("the file cannot be read: {e}")),
    };
    if !f.metadata().is_ok_and(|m| m.is_file()) {
        return unusable("it is not a plain file");
    }
    let mut bytes = Vec::new();
    if let Err(e) = f.take(MAX_INSTRUCTIONS as u64 + 1).read_to_end(&mut bytes) {
        return unusable(&format!("the file cannot be read: {e}"));
    }
    if bytes.len() > MAX_INSTRUCTIONS {
        return unusable(&format!("they are longer than {} KiB: make them shorter", MAX_INSTRUCTIONS / 1024));
    }
    let Ok(text) = String::from_utf8(bytes) else { return unusable("the file is not plain text (UTF-8)") };
    if text.trim().is_empty() {
        return Instructions::None;
    }
    match instructions_ok(&text) {
        Ok(()) => Instructions::Text(text),
        Err(why) => unusable(&why),
    }
}

/// The caller's own instructions.
pub fn mine() -> Instructions {
    path().map_or(Instructions::None, |p| read(&p))
}

/// The file made, empty, if it is not there (its folder too), for an editor to open: only the person's to
/// read and write. An existing one is left as it is.
pub fn create(file: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if let Some(dir) = file.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(file) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

/// The file removed (the link itself, if it is one). Whether there was one.
pub fn clear(file: &Path) -> std::io::Result<bool> {
    match std::fs::remove_file(file) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// What `torq agent instructions` prints: where they are, what they are for, and the text (or that there is none).
pub fn shown(file: &Path, now: &Instructions) -> String {
    let head = format!("Instructions for your agent: {}\n{WHAT_FOR}\n", file.display());
    match now {
        Instructions::None => format!("{head}\nNone yet. `torq agent instructions edit` opens the file in your editor.\n"),
        Instructions::Unusable(why) => format!("{head}\n{why}\n"),
        Instructions::Text(text) => {
            let lines: Vec<String> = text.trim_end().lines().map(|l| format!("  {}", crate::history::clean(l))).collect();
            format!("{head}\nThey are:\n{}\n\nTo change them: `torq agent instructions edit`. To remove them: `torq agent instructions clear`.\n", lines.join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_as_they_stand() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join(".config/torq/agent-instructions.md");
        assert_eq!(read(&f), Instructions::None, "no file: none");
        create(&f).unwrap();
        assert_eq!(read(&f), Instructions::None, "an empty one: none");
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600, "the person's alone");
            assert_eq!(std::fs::metadata(f.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        }
        std::fs::write(&f, " \n\n").unwrap();
        assert_eq!(read(&f), Instructions::None, "only blank lines: none");
        std::fs::write(&f, "Always answer in German.\nExplain commands.\n").unwrap();
        assert_eq!(read(&f), Instructions::Text("Always answer in German.\nExplain commands.\n".into()));
        assert_eq!(read(&f).text(), Some("Always answer in German.\nExplain commands.\n"));
        create(&f).unwrap();
        assert!(read(&f).text().is_some(), "making it again leaves what is there");
        // The longest that is used, and one byte more.
        std::fs::write(&f, "x".repeat(MAX_INSTRUCTIONS)).unwrap();
        assert!(read(&f).text().is_some_and(|t| t.len() == MAX_INSTRUCTIONS));
        std::fs::write(&f, "x".repeat(MAX_INSTRUCTIONS + 1)).unwrap();
        let long = read(&f);
        let said = long.unusable().expect("too long: not used");
        assert!(said.starts_with("Your instructions for your agent are not used: they are longer than 16 KiB: make them shorter (") && said.contains("agent-instructions.md") && said.contains("`torq agent instructions edit`"), "{said}");
        assert_eq!(long.text(), None);
        // Not text; a NUL in it.
        std::fs::write(&f, b"\xff\xfe").unwrap();
        assert!(read(&f).unusable().is_some_and(|s| s.contains("not plain text")));
        std::fs::write(&f, "a\0b").unwrap();
        assert!(read(&f).unusable().is_some_and(|s| s.contains("not text (a NUL)")));
        assert!(clear(&f).unwrap() && !clear(&f).unwrap() && read(&f) == Instructions::None);
    }

    /// Never through a link, never waiting, never an endless read: a link (to a real text, to /dev/zero), a
    /// FIFO, a folder in the file's place are not used, each said, in bounded time.
    #[test]
    fn what_is_not_a_plain_file_is_not_used() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("agent-instructions.md");
        let real = d.path().join("real.md");
        std::fs::write(&real, "Always answer in German.").unwrap();
        let dir = d.path().to_path_buf();
        let (done, over) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let f = dir.join("agent-instructions.md");
            let mut seen = Vec::new();
            std::os::unix::fs::symlink(dir.join("real.md"), &f).unwrap();
            seen.push(read(&f));
            std::fs::remove_file(&f).unwrap();
            std::os::unix::fs::symlink("/dev/zero", &f).unwrap();
            seen.push(read(&f));
            std::fs::remove_file(&f).unwrap();
            rustix::fs::mknodat(rustix::fs::CWD, &f, rustix::fs::FileType::Fifo, rustix::fs::Mode::from_raw_mode(0o600), 0).unwrap();
            seen.push(read(&f));
            std::fs::remove_file(&f).unwrap();
            std::fs::create_dir(&f).unwrap();
            seen.push(read(&f));
            let _ = done.send(seen);
        });
        let seen = over.recv_timeout(std::time::Duration::from_secs(10)).expect("a FIFO or an endless device held the read");
        assert!(seen[0].unusable().is_some_and(|s| s.contains("is a link to another file")), "{:?}", seen[0]);
        assert!(seen[1].unusable().is_some_and(|s| s.contains("is a link to another file")), "{:?}", seen[1]);
        assert!(seen[2].unusable().is_some_and(|s| s.contains("not a plain file")), "{:?}", seen[2]);
        assert!(seen[3].unusable().is_some(), "{:?}", seen[3]);
        assert!(seen.iter().all(|s| s.text().is_none()));
        // The control: the same text in the file itself is used.
        std::fs::remove_dir(&f).unwrap();
        std::fs::copy(&real, &f).unwrap();
        assert_eq!(read(&f).text(), Some("Always answer in German."));
    }

    #[test]
    fn as_torq_agent_instructions_shows_them() {
        let f = Path::new("/home/alice/.config/torq/agent-instructions.md");
        let none = shown(f, &Instructions::None);
        assert!(none.starts_with("Instructions for your agent: /home/alice/.config/torq/agent-instructions.md\n") && none.contains("\"Always answer in German\"") && none.contains("None yet. `torq agent instructions edit`"), "{none}");
        let some = shown(f, &Instructions::Text("Always answer in German.\n\u{1b}[2Jsecond\n".into()));
        assert!(some.contains("They are:\n  Always answer in German.\n") && some.contains("second\n") && !some.contains('\u{1b}') && some.contains("`torq agent instructions clear`"), "{some}");
        let bad = shown(f, &Instructions::Unusable("Your instructions for your agent are not used: x.".into()));
        assert!(bad.contains("are not used: x.") && !bad.contains("They are:"), "{bad}");
    }
}
