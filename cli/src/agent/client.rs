//! The person's own agent, as every program of theirs reaches it (ADR-051 4): `torq ask`, the panel's
//! applet, the agent's window and TorqOS Settings speak to their service (`torq-agent@<you>.service`, on
//! `/run/torq-agent/<you>.sock`) through this one piece. It connects to the caller's own socket only —
//! named by the caller's uid, and refused unless that socket is theirs — and never waits for ever: a
//! service absent, frozen (a share being set up holds it still) or silent is an error in bounded time,
//! so no window or panel thread hangs on it. It holds no secret and runs nothing: what it sends is
//! torq-wire's `agent` requests; what it hands back is the service's events, as they come.

use super::{agent_of, is_agent};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use torq_wire::agent::{socket, Event, Request, MAX_LINE};

/// How long a connection, a request and a short answer (status, stop, forget) may take. Stop waits up to
/// 5 s for a task to end before it answers; the first request may also start the service.
pub const QUICK: Duration = Duration::from_secs(15);
/// How long an answer may be silent between two of its events: the model is asked for at most 600 s a
/// round (agent/src/model.rs), so a little more than that, and the service is then not answering.
pub const QUIET: Duration = Duration::from_secs(660);
/// The most a whole answer may take: a task's rounds are bounded (8 by default, at most 32, each a model's 600 s and its
/// tools'), so this, and then the asker leaves it, which stops it.
pub const ANSWER_MAX: Duration = Duration::from_secs(4 * 3600);

/// Why the agent could not be asked.
#[derive(Debug, Clone, PartialEq)]
pub enum Unreachable {
    /// The caller has no agent account (they are not in `torq.agent.people`).
    NoAgent(String),
    /// The caller is an agent: it has none of its own.
    IsAgent,
    /// No socket: the service is not on this machine (doctor mode off).
    NoService(String),
    /// The socket is there but not the caller's.
    NotYours(String),
    /// The service did not answer in time, or ended before it answered: what it was asked.
    Silent(String),
    /// The service refused or failed: its words.
    Refused(String),
    /// Anything else (the socket's own error).
    Broken(String),
    /// The service could not start for want of the model's key: root's word for it (`KEY_MISSING_DIR`), how
    /// long ago that was said, and the unit whose journal says where the key belongs.
    KeyMissing { ago: Duration, unit: String },
}

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unreachable::NoAgent(agent) => write!(f, "you have no agent account on this machine ({agent}): `torq agent people` names who has one (the owner, by default) — adding you is a system change, an administrator's"),
            Unreachable::IsAgent => write!(f, "this is an agent: it has no agent of its own to ask"),
            Unreachable::NoService(path) => write!(f, "your agent's service is not on this machine ({path}): it runs where doctor mode is (torq agent doctor enable true, an administrator's change)"),
            Unreachable::NotYours(path) => write!(f, "{path}: not yours to connect to"),
            Unreachable::Silent(what) => write!(f, "your agent did not answer {what}"),
            Unreachable::Refused(text) => write!(f, "{text}"),
            Unreachable::Broken(text) => write!(f, "connecting to your agent: {text}"),
            // Said as of when it was found so, not as of now: the key may have been put back since, with no start tried.
            Unreachable::KeyMissing { ago, unit } => write!(f, "your agent cannot start: the model's key is missing (it was when your agent last tried to start, {}); an administrator must install it (journalctl -u {unit} says where, an administrator's)", self::ago(*ago)),
        }
    }
}

impl std::error::Error for Unreachable {}

impl Unreachable {
    /// The panel's one line for it.
    pub fn line(&self) -> String {
        match self {
            Unreachable::NoAgent(_) | Unreachable::IsAgent => "The agent is not set up for you".into(),
            Unreachable::NoService(_) => "The agent is not set up on this machine".into(),
            Unreachable::NotYours(_) => "Not reachable: its socket is not yours".into(),
            Unreachable::Silent(_) => "Not reachable: it does not answer".into(),
            Unreachable::Refused(t) | Unreachable::Broken(t) => format!("Not reachable: {}", crate::history::one_line(t, 80)),
            Unreachable::KeyMissing { .. } => "Cannot start: the model's key is missing (an administrator must install it)".into(),
        }
    }

    /// Whether the person has no agent here at all (no account, or no service), rather than one that failed.
    pub fn not_set_up(&self) -> bool {
        matches!(self, Unreachable::NoAgent(_) | Unreachable::IsAgent | Unreachable::NoService(_))
    }
}

/// Where root leaves word that a person's agent could not start for want of the model's key: an empty file,
/// `<this directory>/<person>`, written by `torq-agent-key@<person>.service` after a failed start and taken
/// away at the next start that gets its key (modules/agent.nix). The key's own directory is root's alone, so
/// a person who is not an administrator could not otherwise be told why their agent is silent. The file
/// holds nothing: that it is there is all it says, and nothing of it is read but whose it is and its age.
pub const KEY_MISSING_DIR: &str = "/run/torq/agent-key-missing";

/// How long the word is waited for after a failure: what writes it starts only once the agent's unit has
/// failed, which may be a moment after this client was turned away.
const KEY_MISSING_WAIT: Duration = Duration::from_secs(1);

/// What is at a path, the path itself (a link not followed): what the word is judged from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seen {
    pub kind: Kind,
    pub uid: u32,
    /// Its permission bits.
    pub mode: u32,
    pub modified: std::time::SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    File,
    Directory,
    /// A link, a socket, a device: never taken as the word.
    Other,
}

/// How long ago `owner` (root) said the key was missing, if they did: a plain file of theirs, in a directory
/// of theirs that no one else can write (so no one else put it there, moved it or linked it). `None` for
/// anything else — no word, or not one to believe. An age the clock makes negative is none at all.
pub fn key_missing_for(dir: Option<Seen>, file: Option<Seen>, owner: u32, now: std::time::SystemTime) -> Option<Duration> {
    let (dir, file) = (dir?, file?);
    let theirs = dir.kind == Kind::Directory && dir.uid == owner && dir.mode & 0o022 == 0;
    (theirs && file.kind == Kind::File && file.uid == owner).then(|| now.duration_since(file.modified).unwrap_or_default())
}

/// A failure, with its reason when root has left one: an agent that is silent, absent or broken while the
/// word is there could not start for want of its key. What the service itself said (it runs, so it has its
/// key), and whose socket or account it is, stay as they are.
pub fn explained(e: Unreachable, key_missing_for: Option<Duration>, unit: &str) -> Unreachable {
    match (e, key_missing_for) {
        (Unreachable::Silent(_) | Unreachable::Broken(_) | Unreachable::NoService(_), Some(ago)) => Unreachable::KeyMissing { ago, unit: unit.to_string() },
        (e, _) => e,
    }
}

/// An age, roughly, as a person reads it.
fn ago(d: Duration) -> String {
    match d.as_secs() {
        s if s < 60 => "less than a minute ago".into(),
        s if s < 120 => "a minute ago".into(),
        s if s < 3600 => format!("{} minutes ago", s / 60),
        s if s < 7200 => "an hour ago".into(),
        s => format!("{} hours ago", s / 3600),
    }
}

/// Where the word for one person's agent is looked for, and whose it must be.
#[derive(Debug, Clone)]
struct Word {
    dir: PathBuf,
    person: String,
    owner: u32,
}

impl Word {
    fn seen(path: &Path) -> Option<Seen> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let m = std::fs::symlink_metadata(path).ok()?;
        let kind = if m.file_type().is_file() {
            Kind::File
        } else if m.file_type().is_dir() {
            Kind::Directory
        } else {
            Kind::Other
        };
        Some(Seen { kind, uid: m.uid(), mode: m.permissions().mode() & 0o7777, modified: m.modified().ok()? })
    }

    /// The word as it is now. A name that is not one file's name in the directory is never looked for.
    fn now(&self) -> Option<Duration> {
        if self.person.is_empty() || self.person.starts_with('.') || self.person.contains('/') {
            return None;
        }
        key_missing_for(Word::seen(&self.dir), Word::seen(&self.dir.join(&self.person)), self.owner, std::time::SystemTime::now())
    }

    /// `e`, with its reason if there is one. Only a failure that could be a start's is looked into, and the
    /// word is waited for (a second at most) only where it could come: its directory is there, root's.
    fn explain(&self, e: Unreachable, unit: &str) -> Unreachable {
        if !matches!(e, Unreachable::Silent(_) | Unreachable::Broken(_) | Unreachable::NoService(_)) {
            return e;
        }
        let until = Instant::now() + KEY_MISSING_WAIT;
        loop {
            let said = self.now();
            let could_come = Word::seen(&self.dir).is_some_and(|d| d.kind == Kind::Directory && d.uid == self.owner);
            if said.is_some() || !could_come || Instant::now() >= until {
                return explained(e, said, unit);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// What `status` says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Status {
    pub running: bool,
    /// How many turns of the conversation it remembers.
    pub turns: usize,
}

/// The caller's own agent: its socket, and whose it must be.
#[derive(Debug, Clone)]
pub struct Client {
    path: PathBuf,
    uid: u32,
    quick: Duration,
    quiet: Duration,
    /// Where root's word of a missing key is looked for, after a failure.
    word: Word,
}

fn has_account(passwd: &str, name: &str) -> bool {
    passwd.lines().any(|l| l.split(':').next() == Some(name))
}

impl Client {
    /// The caller's own agent, by the uid the kernel gives this process: its name, its agent account,
    /// its socket. Refused, plainly, for someone with no agent account, or an agent.
    pub fn mine() -> Result<Client, Unreachable> {
        let uid = rustix::process::getuid().as_raw();
        let passwd = std::fs::read_to_string("/etc/passwd").map_err(|e| Unreachable::Broken(format!("reading /etc/passwd: {e}")))?;
        let me = crate::name_of(&passwd, uid).ok_or_else(|| Unreachable::Broken(format!("uid {uid} has no name in /etc/passwd")))?;
        if is_agent(&me) {
            return Err(Unreachable::IsAgent);
        }
        let agent = agent_of(&me);
        if !has_account(&passwd, &agent) {
            return Err(Unreachable::NoAgent(agent));
        }
        Ok(Client::at(Path::new(&socket(&me)), uid))
    }

    /// The agent on `path`, which must be `uid`'s socket.
    /// The unit that serves this socket: `torq-agent@<person>.service`, the person's own name in it (rather than
    /// a literal <you>).
    pub fn unit(&self) -> String {
        format!("torq-agent@{}.service", self.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
    }

    /// Where an administrator reads why it did not answer.
    fn journal(&self) -> String {
        format!("journalctl -u {}, an administrator's", self.unit())
    }

    pub fn at(path: &Path, uid: u32) -> Client {
        let person = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Client { path: path.to_path_buf(), uid, quick: QUICK, quiet: QUIET, word: Word { dir: PathBuf::from(KEY_MISSING_DIR), person, owner: 0 } }
    }

    /// The same, with root's word of a missing key looked for in another directory, as another owner's (a test's).
    pub fn word_in(self, dir: &Path, owner: u32) -> Client {
        Client { word: Word { dir: dir.to_path_buf(), owner, ..self.word.clone() }, ..self }
    }

    /// The same, with other bounds (a test's).
    pub fn bounded(self, quick: Duration, quiet: Duration) -> Client {
        Client { quick, quiet, ..self }
    }

    /// A connection to the socket, if it is the caller's: within `quick`, even when the service is frozen
    /// and its queue full.
    fn connect(&self) -> Result<UnixStream, Unreachable> {
        use rustix::io::Errno;
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let shown = self.path.display().to_string();
        let m = match std::fs::metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Unreachable::NoService(shown)),
            Err(e) => return Err(Unreachable::Broken(format!("{shown}: {e}"))),
        };
        // Its directory is root's (/run/torq-agent, 0755): no one else can put another socket in its place.
        if !m.file_type().is_socket() || m.uid() != self.uid {
            return Err(Unreachable::NotYours(shown));
        }
        let addr = rustix::net::SocketAddrUnix::new(&self.path).map_err(|e| Unreachable::Broken(format!("{shown}: {e}")))?;
        let fd = rustix::net::socket_with(rustix::net::AddressFamily::UNIX, rustix::net::SocketType::STREAM, rustix::net::SocketFlags::CLOEXEC | rustix::net::SocketFlags::NONBLOCK, None)
            .map_err(|e| Unreachable::Broken(e.to_string()))?;
        let deadline = Instant::now() + self.quick;
        loop {
            match rustix::net::connect(&fd, &addr) {
                Ok(()) => break,
                // Its queue is full (a frozen service): again, until the deadline.
                Err(Errno::AGAIN) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                Err(Errno::AGAIN) => return Err(Unreachable::Silent(format!("in time: it takes no connection ({})", self.journal()))),
                Err(Errno::NOENT) => return Err(Unreachable::NoService(shown)),
                Err(Errno::ACCESS) => return Err(Unreachable::NotYours(shown)),
                Err(Errno::CONNREFUSED) => return Err(Unreachable::Broken(format!("{shown}: nothing listens there (systemctl status {})", self.unit().replace(".service", ".socket")))),
                Err(e) => return Err(Unreachable::Broken(format!("{shown}: {e}"))),
            }
        }
        rustix::io::ioctl_fionbio(&fd, false).map_err(|e| Unreachable::Broken(e.to_string()))?;
        let conn = UnixStream::from(fd);
        conn.set_write_timeout(Some(self.quick)).map_err(|e| Unreachable::Broken(e.to_string()))?;
        conn.set_read_timeout(Some(self.quick)).map_err(|e| Unreachable::Broken(e.to_string()))?;
        Ok(conn)
    }

    /// One request sent on a new connection: the connection, to read its answer from.
    fn send(&self, r: &Request) -> Result<Lines, Unreachable> {
        // A failure to reach it or to ask it, as one to hear it (Lines::next): with its reason, if root left one.
        let why = |e| self.word.explain(e, &self.unit());
        let mut conn = self.connect().map_err(why)?;
        let line = serde_json::to_string(r).map_err(|e| Unreachable::Broken(e.to_string()))?;
        writeln!(conn, "{line}").map_err(|e| why(Unreachable::Broken(format!("asking your agent: {e}"))))?;
        Ok(Lines { r: BufReader::new(conn), deadline: Some(Instant::now() + self.quick), quiet: None, journal: self.journal(), word: self.word.clone(), unit: self.unit() })
    }

    /// Whether a task runs, and how much the conversation keeps.
    pub fn status(&self) -> Result<Status, Unreachable> {
        let mut l = self.send(&Request::Status)?;
        loop {
            match l.next("its status")? {
                Event::Status { running, turns } => return Ok(Status { running, turns }),
                Event::Error { text } => return Err(Unreachable::Refused(crate::history::clean(&text))),
                _ => {}
            }
        }
    }

    /// End the task that runs, whoever asked for it (the person's own, from anywhere): its status after.
    pub fn stop(&self) -> Result<Status, Unreachable> {
        let mut l = self.send(&Request::Stop)?;
        loop {
            match l.next("stop")? {
                Event::Status { running, turns } => return Ok(Status { running, turns }),
                Event::Error { text } => return Err(Unreachable::Refused(crate::history::clean(&text))),
                _ => {}
            }
        }
    }

    /// Erase the conversation it remembers (refused while a task runs).
    pub fn forget(&self) -> Result<(), Unreachable> {
        let mut l = self.send(&Request::Forget)?;
        loop {
            match l.next("forget")? {
                Event::Forgotten => return Ok(()),
                Event::Error { text } => return Err(Unreachable::Refused(crate::history::clean(&text))),
                _ => {}
            }
        }
    }

    /// Ask: the answer's events, as they come, `done` last. Dropping it before then ends the task (the
    /// service stops what no one hears).
    pub fn ask(&self, text: &str, context: Option<String>) -> Result<Answer, Unreachable> {
        let mut lines = self.send(&Request::Ask { text: text.to_string(), context })?;
        // Each of an answer's events comes within `quiet` of the one before, and the whole of it within ANSWER_MAX: a
        // service that keeps it coming never holds the asker for ever.
        lines.deadline = Some(Instant::now() + ANSWER_MAX);
        lines.quiet = Some(self.quiet);
        Ok(Answer { lines, over: false })
    }
}

/// A connection's lines, one event each, each at most `MAX_LINE` bytes; with a deadline, the whole of a
/// short answer is read by it, however slowly its bytes come.
struct Lines {
    r: BufReader<UnixStream>,
    deadline: Option<Instant>,
    /// The most between two events, besides the deadline (an answer's).
    quiet: Option<Duration>,
    /// Where an administrator reads why it is silent.
    journal: String,
    /// Root's word of a missing key, and the unit it is about: why it is silent, when that is why.
    word: Word,
    unit: String,
}

impl Lines {
    /// The next event, or why there is none: `what` was asked. A service that is silent or gone because it
    /// could not start for want of its key is said so (root's word, `KEY_MISSING_DIR`).
    fn next(&mut self, what: &str) -> Result<Event, Unreachable> {
        self.read(what).map_err(|e| self.word.explain(e, &self.unit))
    }

    fn read(&mut self, what: &str) -> Result<Event, Unreachable> {
        let silent = || Unreachable::Silent(format!("{what} in time ({})", self.journal));
        let mut line = Vec::new();
        loop {
            if let Some(d) = self.deadline {
                let left = d.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(silent());
                }
                self.r.get_ref().set_read_timeout(Some(self.quiet.map_or(left, |q| q.min(left)))).map_err(|e| Unreachable::Broken(e.to_string()))?;
            }
            let got = match self.r.fill_buf() {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Err(silent()),
                Err(e) => return Err(Unreachable::Broken(format!("reading your agent's answer: {e}"))),
            };
            if got.is_empty() {
                return Err(Unreachable::Silent(format!("{what}: it ended before it answered ({})", self.journal)));
            }
            let (used, whole) = match got.iter().position(|&b| b == b'\n') {
                Some(i) => (i + 1, true),
                None => (got.len(), false),
            };
            line.extend_from_slice(&got[..used]);
            self.r.consume(used);
            if line.len() > MAX_LINE + 1 {
                return Err(Unreachable::Broken(format!("your agent said something longer than {MAX_LINE} bytes")));
            }
            if whole {
                break;
            }
        }
        serde_json::from_slice(&line).map_err(|_| Unreachable::Broken("your agent said something this torq does not know".into()))
    }
}

/// An answer, streamed: each event as the service says it, ending with `done`; an error ends it too.
pub struct Answer {
    lines: Lines,
    over: bool,
}

impl Iterator for Answer {
    type Item = Result<Event, Unreachable>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.over {
            return None;
        }
        let e = self.lines.next("your question");
        self.over = matches!(e, Ok(Event::Done { .. }) | Err(_));
        Some(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    fn me() -> u32 {
        rustix::process::getuid().as_raw()
    }

    /// A fake service on a socket in a temporary directory: to each connection, after its first line,
    /// the lines `say` gives for it (none: it stays silent, holding the connection). Each request read is
    /// kept, for the test to look at.
    fn service(say: impl Fn(&str) -> Vec<String> + Send + 'static) -> (tempfile::TempDir, PathBuf, std::sync::mpsc::Receiver<String>) {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("me.sock");
        let l = UnixListener::bind(&path).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for conn in l.incoming() {
                let Ok(conn) = conn else { return };
                let mut r = BufReader::new(conn.try_clone().unwrap());
                let mut first = String::new();
                r.read_line(&mut first).unwrap();
                let _ = tx.send(first.trim_end().to_string());
                let lines = say(first.trim_end());
                let mut w = conn;
                if lines.is_empty() {
                    std::thread::sleep(Duration::from_secs(30));
                }
                for l in lines {
                    let _ = writeln!(w, "{l}");
                }
            }
        });
        (d, path, rx)
    }

    fn quick(path: &Path) -> Client {
        // (No word of a missing key, whatever this machine's own /run holds: a directory that is not there.)
        Client::at(path, me()).bounded(Duration::from_millis(500), Duration::from_millis(500)).word_in(Path::new("/nonexistent/torq-agent-key-missing"), me())
    }

    /// An answer streamed: each event of the task, `done` last, then nothing; the request as the wire says.
    #[test]
    fn an_answer_streamed() {
        let (_d, path, asked) = service(|_| {
            [
                r#"{"type":"called","tool":"read_journal"}"#,
                r#"{"type":"tainted","why":"read_journal"}"#,
                r#"{"type":"say","text":"fine"}"#,
                r#"{"type":"proposed","seq":7}"#,
                r#"{"type":"done","exit":0}"#,
                r#"{"type":"say","text":"after done"}"#,
            ]
            .map(String::from)
            .to_vec()
        });
        let got: Vec<_> = quick(&path).ask("why?", None).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(
            got,
            vec![
                Event::Called { tool: "read_journal".into() },
                Event::Tainted { why: "read_journal".into() },
                Event::Say { text: "fine".into() },
                Event::Proposed { seq: 7 },
                Event::Done { exit: 0 },
            ],
            "nothing after done"
        );
        assert_eq!(asked.recv().unwrap(), r#"{"type":"ask","text":"why?"}"#);
    }

    #[test]
    fn status_stop_and_forget() {
        let (_d, path, asked) = service(|req| match req {
            r#"{"type":"status"}"# => vec![r#"{"type":"status","running":true,"turns":4}"#.into()],
            r#"{"type":"stop"}"# => vec![r#"{"type":"status","running":false,"turns":4}"#.into()],
            _ => vec![r#"{"type":"error","text":"a task is running: stop it first\u001b[2J"}"#.into(), r#"{"type":"done","exit":1}"#.into()],
        });
        let c = quick(&path);
        assert_eq!(c.status().unwrap(), Status { running: true, turns: 4 });
        assert_eq!(c.stop().unwrap(), Status { running: false, turns: 4 });
        let refused = c.forget().unwrap_err();
        assert!(matches!(&refused, Unreachable::Refused(t) if t.starts_with("a task is running") && !t.contains('\u{1b}')), "{refused:?}");
        let sent: Vec<String> = asked.try_iter().collect();
        assert_eq!(sent, [r#"{"type":"status"}"#, r#"{"type":"stop"}"#, r#"{"type":"forget"}"#]);
    }

    /// A service that never answers: every call ends, in its bound.
    #[test]
    fn a_silent_service_times_out() {
        let (_d, path, _asked) = service(|_| vec![]);
        let c = quick(&path);
        let t = Instant::now();
        assert!(matches!(c.status(), Err(Unreachable::Silent(_))));
        let mut a = c.ask("hello", None).unwrap();
        let first = a.next();
        assert!(matches!(first, Some(Err(Unreachable::Silent(_)))), "{first:?}");
        assert!(a.next().is_none(), "over at its error");
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    }

    /// A service that ends without its last word: an error, not an answer.
    #[test]
    fn a_service_that_ends_early() {
        let (_d, path, _asked) = service(|_| vec![r#"{"type":"say","text":"half"}"#.into()]);
        let got: Vec<_> = quick(&path).ask("q", None).unwrap().collect();
        assert_eq!(got[0], Ok(Event::Say { text: "half".into() }));
        assert!(matches!(got[1], Err(Unreachable::Silent(_))) && got.len() == 2, "{got:?}");
    }

    /// Only the caller's own socket: one of another uid is refused before anything is sent; no socket is
    /// no service; a file that is not a socket is not theirs either.
    #[test]
    fn only_ones_own_socket() {
        let (d, path, asked) = service(|_| vec![r#"{"type":"status","running":false,"turns":0}"#.into()]);
        let other = Client::at(&path, me() + 1).bounded(Duration::from_millis(500), Duration::from_millis(500));
        assert_eq!(other.status().unwrap_err(), Unreachable::NotYours(path.display().to_string()));
        assert!(other.ask("hi", None).is_err());
        assert!(asked.recv_timeout(Duration::from_millis(200)).is_err(), "nothing was sent");
        assert!(matches!(quick(&d.path().join("none.sock")).status(), Err(Unreachable::NoService(_))));
        std::fs::write(d.path().join("file"), "").unwrap();
        assert!(matches!(quick(&d.path().join("file")).status(), Err(Unreachable::NotYours(_))));
        assert!(quick(&path).status().unwrap() == Status { running: false, turns: 0 }, "one's own is asked");
    }

    /// A socket no one listens on any more: an error at once, not a wait.
    #[test]
    fn nothing_listening() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("gone.sock");
        drop(UnixListener::bind(&path).unwrap());
        assert!(matches!(quick(&path).status(), Err(Unreachable::Broken(t)) if t.contains("nothing listens")));
    }

    fn seen(kind: Kind, uid: u32, mode: u32, secs_ago: u64, now: std::time::SystemTime) -> Option<Seen> {
        Some(Seen { kind, uid, mode, modified: now - Duration::from_secs(secs_ago) })
    }

    /// Root's word of a missing key is believed only as root's: a plain file of root's, in a directory of
    /// root's that no one else can write. Anything else — nothing there, a link, another's file, a directory
    /// others can write, a directory that is itself a link — is no word.
    #[test]
    fn roots_word_of_a_missing_key_and_nothing_elses() {
        let now = std::time::SystemTime::now();
        let dir = seen(Kind::Directory, 0, 0o755, 3600, now);
        let file = seen(Kind::File, 0, 0o644, 90, now);
        assert_eq!(key_missing_for(dir, file, 0, now), Some(Duration::from_secs(90)));
        // No word: no file, no directory.
        assert_eq!(key_missing_for(dir, None, 0, now), None);
        assert_eq!(key_missing_for(None, file, 0, now), None);
        // Not root's file; a link or anything else in its place (never followed); a directory in its place.
        assert_eq!(key_missing_for(dir, seen(Kind::File, 1000, 0o644, 90, now), 0, now), None);
        assert_eq!(key_missing_for(dir, seen(Kind::Other, 0, 0o777, 90, now), 0, now), None);
        assert_eq!(key_missing_for(dir, seen(Kind::Directory, 0, 0o755, 90, now), 0, now), None);
        // Its directory another's, or one others can write (group or anyone: a file there could be moved or
        // replaced), or not a directory (a link to one).
        assert_eq!(key_missing_for(seen(Kind::Directory, 1000, 0o755, 0, now), file, 0, now), None);
        assert_eq!(key_missing_for(seen(Kind::Directory, 0, 0o775, 0, now), file, 0, now), None);
        assert_eq!(key_missing_for(seen(Kind::Directory, 0, 0o1777, 0, now), file, 0, now), None);
        assert_eq!(key_missing_for(seen(Kind::Other, 0, 0o755, 0, now), file, 0, now), None);
        // The file's own mode is not what is trusted (its directory is): root's is root's.
        assert!(key_missing_for(dir, seen(Kind::File, 0, 0o600, 5, now), 0, now).is_some());
        // A clock set back since: no negative age, said as just now.
        let ahead = Some(Seen { kind: Kind::File, uid: 0, mode: 0o644, modified: now + Duration::from_secs(600) });
        assert_eq!(key_missing_for(dir, ahead, 0, now), Some(Duration::ZERO));
    }

    /// The client's decision: an agent that is silent, gone or broken while root's word is there could not
    /// start for want of its key, and is said so with the unit's journal; with no word, each failure is as it
    /// was. What the service itself said, and whose it is, is never called a missing key.
    #[test]
    fn a_failure_is_explained_by_roots_word_or_left_as_it_was() {
        let unit = "torq-agent@alice.service";
        let word = Some(Duration::from_secs(150));
        let missing = Unreachable::KeyMissing { ago: Duration::from_secs(150), unit: unit.into() };
        for e in [Unreachable::Silent("your question: it ended before it answered".into()), Unreachable::Broken("reading your agent's answer: reset".into()), Unreachable::NoService("/run/torq-agent/alice.sock".into())] {
            assert_eq!(explained(e.clone(), word, unit), missing, "{e:?}");
            assert_eq!(explained(e.clone(), None, unit), e, "no word: as it was");
        }
        for e in [Unreachable::Refused("a task is running: stop it first".into()), Unreachable::NotYours("/x".into()), Unreachable::NoAgent("torq-agent-alice".into()), Unreachable::IsAgent] {
            assert_eq!(explained(e.clone(), word, unit), e, "{e:?}");
        }
        // What the person reads: the reason, whose it is to put right, where the journal is — and as of when.
        let said = missing.to_string();
        assert!(said.contains("the model's key is missing") && said.contains("an administrator must install it") && said.contains("journalctl -u torq-agent@alice.service"), "{said}");
        assert!(said.contains("2 minutes ago") && !said.contains("did not answer"), "{said}");
        assert_eq!(missing.line(), "Cannot start: the model's key is missing (an administrator must install it)");
        assert!(!missing.not_set_up(), "set up, and failing: the panel's mark is trouble");
        assert_eq!([0, 59, 60, 119, 120, 3599, 3600, 7200].map(|s| ago(Duration::from_secs(s))), ["less than a minute ago", "less than a minute ago", "a minute ago", "a minute ago", "2 minutes ago", "59 minutes ago", "an hour ago", "2 hours ago"]);
    }

    /// The same through the files themselves: a silent service and a word in a directory the owner alone
    /// writes is a missing key; the word gone, or a link in its place, or a directory others can write, it is
    /// silence again. Nothing waits long for a word that cannot come.
    #[test]
    fn the_word_read_from_its_directory() {
        use std::os::unix::fs::PermissionsExt;
        let (d, path, _asked) = service(|_| vec![]);
        let words = d.path().join("agent-key-missing");
        std::fs::create_dir(&words).unwrap();
        std::fs::set_permissions(&words, std::fs::Permissions::from_mode(0o755)).unwrap();
        let c = || Client::at(&path, me()).bounded(Duration::from_millis(300), Duration::from_millis(300)).word_in(&words, me());
        // (The socket is me.sock: the person is "me".)
        std::fs::write(words.join("me"), "").unwrap();
        assert!(matches!(c().status(), Err(Unreachable::KeyMissing { unit, .. }) if unit == "torq-agent@me.service"));
        let first = c().ask("hello", None).unwrap().next();
        assert!(matches!(first, Some(Err(Unreachable::KeyMissing { .. }))), "{first:?}");
        // No socket at all (it stopped after too many failed starts): the same reason.
        let gone = Client::at(&d.path().join("none.sock"), me()).bounded(Duration::from_millis(300), Duration::from_millis(300)).word_in(&words, me());
        std::fs::write(words.join("none"), "").unwrap();
        assert!(matches!(gone.status(), Err(Unreachable::KeyMissing { .. })));
        // Another person's word is not this one's.
        std::fs::remove_file(words.join("me")).unwrap();
        std::fs::write(words.join("bob"), "").unwrap();
        assert!(matches!(c().status(), Err(Unreachable::Silent(_))));
        // A link in the word's place is not followed, wherever it goes.
        std::os::unix::fs::symlink(words.join("bob"), words.join("me")).unwrap();
        assert!(matches!(c().status(), Err(Unreachable::Silent(_))));
        std::fs::remove_file(words.join("me")).unwrap();
        // A directory others can write: a word there is anyone's.
        std::fs::write(words.join("me"), "").unwrap();
        std::fs::set_permissions(&words, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(matches!(c().status(), Err(Unreachable::Silent(_))));
        // Not the owner asked for (root, in the real one): no word.
        std::fs::set_permissions(&words, std::fs::Permissions::from_mode(0o755)).unwrap();
        let not_roots = Client::at(&path, me()).bounded(Duration::from_millis(300), Duration::from_millis(300)).word_in(&words, me() + 1);
        let t = Instant::now();
        assert!(matches!(not_roots.status(), Err(Unreachable::Silent(_))));
        assert!(t.elapsed() < Duration::from_millis(1200), "no wait for a word where none can come: {:?}", t.elapsed());
        // A service that answers is never said to lack its key, whatever word was left behind.
        let (_d2, answering, _asked2) = service(|_| vec![r#"{"type":"error","text":"a task is running: stop it first"}"#.into()]);
        let words2 = d.path().join("stale");
        std::fs::create_dir(&words2).unwrap();
        std::fs::set_permissions(&words2, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(words2.join("me"), "").unwrap();
        let stale = Client::at(&answering, me()).bounded(Duration::from_millis(300), Duration::from_millis(300)).word_in(&words2, me());
        assert!(matches!(stale.forget(), Err(Unreachable::Refused(_))));
        // The real client looks where the module writes (modules/agent.nix, tests/agent-assertions.nix), for root's.
        let real = Client::at(Path::new("/run/torq-agent/alice.sock"), 1000);
        assert_eq!((real.word.dir.to_str(), real.word.person.as_str(), real.word.owner), (Some("/run/torq/agent-key-missing"), "alice", 0));
    }

    /// A service whose queue is full (frozen, never accepting): connecting gives up at its bound.
    #[test]
    fn a_frozen_service_is_given_up_on() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("frozen.sock");
        let fd = rustix::net::socket(rustix::net::AddressFamily::UNIX, rustix::net::SocketType::STREAM, None).unwrap();
        rustix::net::bind(&fd, &rustix::net::SocketAddrUnix::new(&path).unwrap()).unwrap();
        rustix::net::listen(&fd, 0).unwrap();
        // Fill its queue: connections no one accepts.
        let mut held = vec![];
        for _ in 0..8 {
            let s = rustix::net::socket_with(rustix::net::AddressFamily::UNIX, rustix::net::SocketType::STREAM, rustix::net::SocketFlags::NONBLOCK, None).unwrap();
            if rustix::net::connect(&s, &rustix::net::SocketAddrUnix::new(&path).unwrap()).is_err() {
                break;
            }
            held.push(s);
        }
        let t = Instant::now();
        assert!(matches!(quick(&path).status(), Err(Unreachable::Silent(_))));
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    }

    #[test]
    fn whose_name_is_this_uid() {
        let passwd = "root:x:0:0::/root:/bin/sh\nalice:x:1000:100::/home/alice:/bin/sh\ntorq-agent-alice:x:990:990::/var/lib/torq-agent-alice:/sbin/nologin\n";
        assert_eq!(crate::name_of(passwd, 1000).as_deref(), Some("alice"));
        assert_eq!(crate::name_of(passwd, 1001), None);
        assert!(has_account(passwd, "torq-agent-alice") && !has_account(passwd, "torq-agent-bob"));
    }

    #[test]
    fn what_the_panel_says_of_each() {
        assert_eq!(Unreachable::NoAgent("torq-agent-me".into()).line(), "The agent is not set up for you");
        assert!(Unreachable::NoService("/x".into()).not_set_up() && !Unreachable::Silent("x".into()).not_set_up());
        assert!(!Unreachable::Refused("a\nb\u{1b}".into()).line().contains(['\n', '\u{1b}']));
    }

    /// A service that trickles its answer a byte at a time, never ending the line: one deadline for the
    /// whole short answer, not one for each byte.
    #[test]
    fn a_trickling_service_is_given_up_on() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("slow.sock");
        let l = UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            if let Ok((mut c, _)) = l.accept() {
                for _ in 0..200 {
                    if c.write_all(b" ").is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });
        let t = Instant::now();
        assert!(matches!(quick(&path).status(), Err(Unreachable::Silent(_))));
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    }
}
