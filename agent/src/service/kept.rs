//! What the agent's service keeps between its starts (ADR-055 3): which conversation of the engine's is the
//! person's (the engine's own name for it, so a restart continues it), how many questions it has had, and the
//! conversation's number (it changes at every forget, so a window that saw another one knows), and a mark of the
//! standing instructions it began with (so a changed text is known to be another). Two small files
//! in the service's state directory (`/var/lib/torq-agent-memory/<person>`, 0700, the agent's). Nothing of
//! what was said is here: the engine keeps its own conversations, in the agent's home.
//!
//! A file that is not what this wrote (too large, not JSON, a link, a name that is not shaped like a
//! session's) is taken as nothing kept: the next question starts a new conversation.

use super::engine::session_ok;
use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The largest file read; anything larger is not this program's.
const MAX_FILE: u64 = 4096;

/// The most questions a conversation kept may say it has had: a count above it is not this program's.
const MAX_QUESTIONS: u64 = 1_000_000;

/// The text of `file`, if it is one this could have written: none if it is not there; an error if it is a
/// link, not a regular file, larger than `MAX_FILE`, or not text. The engine has this program's uid and can
/// write the state directory, so what is found there is never trusted to be a file: it is opened without
/// following a link and without waiting (a FIFO put there would otherwise hold the service at its open for
/// good), and read only if it is a regular file, and then no further than the bound (a link to an endless
/// device would otherwise be read until the memory was gone).
fn read_own(file: &Path) -> Result<Option<String>> {
    use rustix::fs::OFlags;
    use std::os::unix::fs::OpenOptionsExt;
    let f = match std::fs::OpenOptions::new().read(true).custom_flags((OFlags::NOFOLLOW | OFlags::NONBLOCK).bits() as i32).open(file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        f => f?,
    };
    anyhow::ensure!(f.metadata()?.is_file(), "not a regular file");
    let mut text = String::new();
    f.take(MAX_FILE + 1).read_to_string(&mut text)?;
    anyhow::ensure!(text.len() as u64 <= MAX_FILE, "larger than {MAX_FILE} bytes");
    Ok(Some(text))
}

/// The conversation as it stands.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Conversation {
    /// The engine's name for it, once the engine has said one.
    pub session: Option<String>,
    /// How many questions it has had.
    pub questions: usize,
    /// A mark of the person's standing instructions the conversation began with (`mark`; none: it began with
    /// none). The engine records what it was given at a conversation's first question and keeps to that
    /// record when the conversation is continued, so the service must know whether today's instructions are
    /// still those. Not the text: only whether it is the same.
    pub began: Option<String>,
}

pub struct Kept {
    dir: PathBuf,
}

impl Kept {
    pub fn new(dir: &Path) -> Kept {
        Kept { dir: dir.to_path_buf() }
    }

    fn file(&self) -> PathBuf {
        self.dir.join("conversation.json")
    }

    fn generation_file(&self) -> PathBuf {
        self.dir.join("generation")
    }

    /// The conversation kept: none (a new one) if nothing is kept, or it is not what this wrote.
    pub fn conversation(&self) -> Conversation {
        let read = || -> Result<Conversation> {
            let Some(text) = read_own(&self.file())? else { return Ok(Conversation::default()) };
            let v: serde_json::Value = serde_json::from_str(&text)?;
            let session = match &v["session"] {
                serde_json::Value::Null => None,
                serde_json::Value::String(s) if session_ok(s) => Some(s.clone()),
                _ => anyhow::bail!("its session is not a session's name"),
            };
            let questions = v["questions"].as_u64().context("no count of its questions")?;
            anyhow::ensure!(questions <= MAX_QUESTIONS, "its count of questions is more than {MAX_QUESTIONS}");
            let began = match &v["began"] {
                serde_json::Value::Null => None,
                serde_json::Value::String(m) if m.len() == 16 && m.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) => Some(m.clone()),
                _ => anyhow::bail!("its mark of the instructions is not one"),
            };
            Ok(Conversation { session, questions: questions as usize, began })
        };
        read().unwrap_or_else(|e| {
            eprintln!("torq-agent-service: {}: not read ({e:#}); the next question starts a new conversation", self.file().display());
            Conversation::default()
        })
    }

    /// Keep `c`. Written whole, then put in place: a crash leaves the old one or the new, never half of one.
    pub fn keep(&self, c: &Conversation) -> Result<()> {
        write_whole(&self.file(), serde_json::json!({ "session": c.session, "questions": c.questions, "began": c.began }).to_string().as_bytes())
    }

    /// Which conversation this is (never 0): it changes at every `forget`. A counter in its own file; a
    /// missing or unreadable one is the first.
    pub fn generation(&self) -> u64 {
        read_own(&self.generation_file()).ok().flatten().and_then(|t| t.trim().parse::<u64>().ok()).filter(|&g| g > 0).unwrap_or(1)
    }

    /// Forget the conversation: the next question starts a new one, under a new number.
    pub fn forget(&self) -> Result<()> {
        // The new number first: a failure leaves the conversation in place, and a window told "another
        // conversation" for one not forgotten is only reset, never misled.
        write_whole(&self.generation_file(), self.generation().saturating_add(1).to_string().as_bytes())?;
        match std::fs::remove_file(self.file()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e).with_context(|| format!("removing {}", self.file().display())),
            _ => Ok(()),
        }
    }
}

/// A mark of `text` (a person's standing instructions): sixteen hexadecimal digits, the same for the same
/// text for ever (FNV-1a, 64 bits). Two different texts have the same mark only by a freak chance, and
/// nothing rests on it but which instructions a continued conversation is given.
pub fn mark(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// `bytes` as the whole of `file`: a new file beside it (never written through a link), then renamed over it.
fn write_whole(file: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let new = file.with_extension("new");
    let _ = std::fs::remove_file(&new);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(&new)
        .with_context(|| format!("writing {}", new.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&new, file).with_context(|| format!("writing {}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation(session: Option<&str>, questions: usize) -> Conversation {
        Conversation { session: session.map(String::from), questions, began: None }
    }

    #[test]
    fn kept_across_a_restart_and_forgotten() {
        let d = tempfile::tempdir().unwrap();
        let k = Kept::new(d.path());
        assert_eq!(k.conversation(), Conversation::default(), "none at first");
        k.keep(&conversation(Some("ef37b4eb-a4bb-45a9-aa2a-db4f61c11dcb"), 3)).unwrap();
        assert_eq!(Kept::new(d.path()).conversation(), conversation(Some("ef37b4eb-a4bb-45a9-aa2a-db4f61c11dcb"), 3));
        k.keep(&conversation(None, 1)).unwrap();
        assert_eq!(k.conversation(), conversation(None, 1));
        k.forget().unwrap();
        assert_eq!(k.conversation(), Conversation::default());
        k.forget().unwrap();
    }

    #[test]
    fn what_is_not_its_own_is_nothing_kept() {
        let d = tempfile::tempdir().unwrap();
        let k = Kept::new(d.path());
        let f = d.path().join("conversation.json");
        for bad in ["not json", r#"{"session":"--help","questions":1}"#, r#"{"session":"a b","questions":1}"#, r#"{"session":7,"questions":1}"#, r#"{"session":"abc"}"#, r#"[{"question":"q","answer":null}]"#] {
            std::fs::write(&f, bad).unwrap();
            assert_eq!(k.conversation(), Conversation::default(), "{bad}");
        }
        std::fs::write(&f, format!(r#"{{"session":"abc","questions":1,"more":"{}"}}"#, "x".repeat(MAX_FILE as usize))).unwrap();
        assert_eq!(k.conversation(), Conversation::default(), "larger than its bound: not read");
        std::fs::remove_file(&f).unwrap();
        let elsewhere = d.path().join("elsewhere.json");
        std::fs::write(&elsewhere, r#"{"session":"planted","questions":9}"#).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &f).unwrap();
        assert_eq!(k.conversation(), Conversation::default(), "never through a link");
        // The control: the same words in its own file are read.
        std::fs::remove_file(&f).unwrap();
        std::fs::copy(&elsewhere, &f).unwrap();
        assert_eq!(k.conversation(), conversation(Some("planted"), 9));
        // A count no conversation has: not its own. At the bound, it is.
        std::fs::write(&f, format!(r#"{{"session":"abc","questions":{}}}"#, MAX_QUESTIONS + 1)).unwrap();
        assert_eq!(k.conversation(), Conversation::default(), "a count above the bound");
        std::fs::write(&f, r#"{"session":"abc","questions":18446744073709551615}"#).unwrap();
        assert_eq!(k.conversation(), Conversation::default(), "the largest number there is");
        std::fs::write(&f, format!(r#"{{"session":"abc","questions":{MAX_QUESTIONS}}}"#)).unwrap();
        assert_eq!(k.conversation(), conversation(Some("abc"), MAX_QUESTIONS as usize), "the control: at the bound");
        // What the engine (the same uid) could put in the state directory in place of either file: a FIFO no
        // one writes to, a link to a device that never ends, a directory. None is waited for, none is read.
        let g = d.path().join("generation");
        let (done, over) = std::sync::mpsc::channel();
        let dir = d.path().to_path_buf();
        std::thread::spawn(move || {
            let k = Kept::new(&dir);
            let (f, g) = (dir.join("conversation.json"), dir.join("generation"));
            let mut seen = Vec::new();
            for planted in [&f, &g] {
                let _ = std::fs::remove_file(planted);
                rustix::fs::mknodat(rustix::fs::CWD, planted, rustix::fs::FileType::Fifo, rustix::fs::Mode::from_raw_mode(0o600), 0).unwrap();
            }
            let fifo = read_own(&f).unwrap_err().to_string();
            seen.push((k.conversation(), k.generation()));
            for planted in [&f, &g] {
                std::fs::remove_file(planted).unwrap();
                std::os::unix::fs::symlink("/dev/zero", planted).unwrap();
            }
            seen.push((k.conversation(), k.generation()));
            for planted in [&f, &g] {
                std::fs::remove_file(planted).unwrap();
                std::fs::create_dir(planted).unwrap();
            }
            seen.push((k.conversation(), k.generation()));
            let _ = done.send((seen, fifo));
        });
        let (seen, fifo) = over.recv_timeout(std::time::Duration::from_secs(10)).expect("a FIFO or an endless device in the state directory held the service");
        assert_eq!(fifo, "not a regular file", "told by what it is, before anything of it is read");
        assert_eq!(seen, vec![(Conversation::default(), 1); 3], "a FIFO, a link to /dev/zero, a directory: nothing kept, the first conversation");
        // The control: a number in its own file is read.
        std::fs::remove_dir(&g).unwrap();
        std::fs::write(&g, "7\n").unwrap();
        assert_eq!(k.generation(), 7);
        std::fs::write(&g, "9".repeat(MAX_FILE as usize + 1)).unwrap();
        assert_eq!(k.generation(), 1, "larger than its bound: not read");
    }

    /// The mark of the instructions a conversation began with is kept with it, and is nothing but a mark: the
    /// same for the same text, another for another, never the text; one that is not a mark is not this program's.
    #[test]
    fn the_instructions_a_conversation_began_with_are_marked() {
        assert_eq!(mark(""), "cbf29ce484222325");
        assert_eq!(mark("a"), "af63dc4c8601ec8c");
        assert_eq!(mark("Always answer in German."), mark("Always answer in German."));
        assert_ne!(mark("Always answer in German."), mark("Always answer in French."));
        let d = tempfile::tempdir().unwrap();
        let k = Kept::new(d.path());
        let began = Conversation { session: Some("abc".into()), questions: 2, began: Some(mark("Always answer in German.")) };
        k.keep(&began).unwrap();
        assert_eq!(Kept::new(d.path()).conversation(), began);
        let text = std::fs::read_to_string(d.path().join("conversation.json")).unwrap();
        assert!(!text.contains("German"), "the mark, not the text: {text}");
        // One kept before there were instructions (no such field): it began with none.
        std::fs::write(d.path().join("conversation.json"), r#"{"session":"abc","questions":2}"#).unwrap();
        assert_eq!(k.conversation(), conversation(Some("abc"), 2));
        for bad in [r#"{"session":"abc","questions":2,"began":"Always answer in German."}"#, r#"{"session":"abc","questions":2,"began":7}"#, r#"{"session":"abc","questions":2,"began":"CBF29CE484222325"}"#] {
            std::fs::write(d.path().join("conversation.json"), bad).unwrap();
            assert_eq!(k.conversation(), Conversation::default(), "{bad}");
        }
    }

    /// The conversation's number changes at every forget and at nothing else.
    #[test]
    fn the_conversations_number_changes_at_a_forget() {
        let d = tempfile::tempdir().unwrap();
        let k = Kept::new(d.path());
        assert_eq!(k.generation(), 1, "the first");
        k.keep(&conversation(Some("abc"), 1)).unwrap();
        assert_eq!(k.generation(), 1, "a question does not change it");
        k.forget().unwrap();
        assert_eq!(k.generation(), 2);
        assert_eq!(Kept::new(d.path()).generation(), 2, "it survives a restart");
        k.forget().unwrap();
        assert_eq!(k.generation(), 3, "a forget of nothing is one too: a window is only reset");
        std::fs::write(d.path().join("generation"), "junk").unwrap();
        assert_eq!(k.generation(), 1, "unreadable: the first, never 0");
    }
}
