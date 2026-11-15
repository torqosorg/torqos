//! What the agent's service keeps between its starts (ADR-055 3): which conversation of the engine's is the
//! person's (the engine's own name for it, so a restart continues it), how many questions it has had, and the
//! conversation's number (it changes at every forget, so a window that saw another one knows). Two small files
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

/// The conversation as it stands.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Conversation {
    /// The engine's name for it, once the engine has said one.
    pub session: Option<String>,
    /// How many questions it has had.
    pub questions: usize,
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
            use std::os::unix::fs::OpenOptionsExt;
            let f = match std::fs::OpenOptions::new().read(true).custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32).open(self.file()) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Conversation::default()),
                f => f?,
            };
            let mut text = String::new();
            f.take(MAX_FILE + 1).read_to_string(&mut text)?;
            anyhow::ensure!(text.len() as u64 <= MAX_FILE, "larger than {MAX_FILE} bytes");
            let v: serde_json::Value = serde_json::from_str(&text)?;
            let session = match &v["session"] {
                serde_json::Value::Null => None,
                serde_json::Value::String(s) if session_ok(s) => Some(s.clone()),
                _ => anyhow::bail!("its session is not a session's name"),
            };
            let questions = v["questions"].as_u64().context("no count of its questions")? as usize;
            Ok(Conversation { session, questions })
        };
        read().unwrap_or_else(|e| {
            eprintln!("torq-agent-service: {}: not read ({e:#}); the next question starts a new conversation", self.file().display());
            Conversation::default()
        })
    }

    /// Keep `c`. Written whole, then put in place: a crash leaves the old one or the new, never half of one.
    pub fn keep(&self, c: &Conversation) -> Result<()> {
        write_whole(&self.file(), serde_json::json!({ "session": c.session, "questions": c.questions }).to_string().as_bytes())
    }

    /// Which conversation this is (never 0): it changes at every `forget`. A counter in its own file; a
    /// missing or unreadable one is the first.
    pub fn generation(&self) -> u64 {
        std::fs::read_to_string(self.generation_file()).ok().and_then(|t| t.trim().parse::<u64>().ok()).filter(|&g| g > 0).unwrap_or(1)
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
        Conversation { session: session.map(String::from), questions }
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
