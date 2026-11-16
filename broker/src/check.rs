//! The check of an agent's proposal (ADR-056): the broker has the change evaluated before it records it.
//!
//! An agent's account is refused at the Nix daemon, so it cannot evaluate what it proposes. The broker
//! does not evaluate either: root never reads an agent's change as Nix, and never unpacks it. It starts a
//! checker (torqcli's `torq --check-tree`, as the locked account `torq-check` in a unit of its own: the
//! command is the machine's setting, modules/broker.nix) and passes it one thing, the id of the git tree
//! its own vetting made. This file is all the broker does with the checker: start it, wait a bounded time,
//! read a bounded answer, and take from it only what has exactly the expected shape.
//!
//! The checker's exit status says nothing here: a unit killed for its time or its memory ends as any
//! failure does. Only its own line counts, and anything else is "no answer", never "not valid".

use serde::Deserialize;
use std::io::Read as _;
use std::process::Stdio;
use std::time::{Duration, Instant};

/// How much of the checker's output is read: its one line is far shorter. More is no answer.
pub const OUTPUT_MAX: u64 = 64 * 1024;
/// How many of the configuration's warnings are kept with a proposal.
pub const WARNINGS_MAX: usize = 8;
/// How long a warning, or the reason a change is not valid, may be: cut there.
pub const TEXT_MAX: usize = 300;
/// How long the broker waits for the check of one proposal, the change and (only if the change fails) the
/// configuration as it is, together. The checker's unit stops itself sooner (`RuntimeMaxSec`).
pub const WAIT: Duration = Duration::from_secs(320);
/// How long a proposal waits for another's check to end before it is turned away as busy.
pub const TURN: u64 = 30;

/// What the checker said of one tree.
#[derive(Debug, PartialEq)]
pub enum Said {
    /// The configuration evaluates; its warnings.
    Valid(Vec<String>),
    /// It does not, and what is wrong.
    NotValid(String),
    /// No answer, and why, in words for a person: it could not be started, did not end in time, or wrote
    /// something else.
    Nothing(&'static str),
}

pub const NOT_STARTED: &str = "the checker could not be started";
pub const NOT_IN_TIME: &str = "the check did not finish in time";
pub const NO_ANSWER: &str = "the checker gave no answer";

/// A git tree's id, in full: 40 lowercase hex digits. The only thing put on the checker's command line.
pub fn tree_id(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The checker's words as the ledger keeps them: one line (a control character is a `?`, as in a
/// refusal's reason), no longer than `TEXT_MAX`.
fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { '?' } else { c }).take(TEXT_MAX).collect()
}

/// The checker's line, and nothing else in it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Line {
    ok: bool,
    warnings: Option<Vec<String>>,
    why: Option<String>,
}

/// The checker's output read strictly: exactly one line of JSON, `{"ok":true,"warnings":[…]}` or
/// `{"ok":false,"why":"…"}`. Anything else is no answer.
pub fn parse(output: &[u8]) -> Said {
    let Ok(text) = std::str::from_utf8(output) else { return Said::Nothing(NO_ANSWER) };
    let line = text.strip_suffix('\n').unwrap_or(text);
    if line.contains('\n') {
        return Said::Nothing(NO_ANSWER);
    }
    match serde_json::from_str::<Line>(line) {
        Ok(Line { ok: true, warnings: Some(w), why: None }) => Said::Valid(w.iter().take(WARNINGS_MAX).map(|w| one_line(w)).collect()),
        Ok(Line { ok: false, warnings: None, why: Some(why) }) if !why.trim().is_empty() => Said::NotValid(one_line(&why)),
        _ => Said::Nothing(NO_ANSWER),
    }
}

/// Run the checker (`command`, a shell command with the tree's id already in it) and read what it says,
/// waiting no longer than `until`. It gets no input and its errors go nowhere: they are Nix's own words,
/// and no one's answer. Whatever happens, it is stopped (it and what it started here) before this returns.
pub fn run(command: &str, until: Instant) -> Said {
    use std::os::unix::process::CommandExt as _;
    let mut c = crate::config::command("sh");
    // A group of its own, so that what the shell started ends with it.
    c.arg("-c").arg(command).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).process_group(0);
    let Ok(mut child) = c.spawn() else { return Said::Nothing(NOT_STARTED) };
    let Some(out) = child.stdout.take() else { return Said::Nothing(NOT_STARTED) };
    let (told, tell) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut read = Vec::new();
        // One byte past the most: more than that is seen as more, and not read.
        let whole = out.take(OUTPUT_MAX + 1).read_to_end(&mut read).is_ok();
        let _ = told.send(whole.then_some(read));
    });
    let said = match tell.recv_timeout(until.saturating_duration_since(Instant::now())) {
        Ok(Some(read)) if read.len() as u64 <= OUTPUT_MAX => parse(&read),
        Ok(_) => Said::Nothing(NO_ANSWER),
        Err(_) => Said::Nothing(NOT_IN_TIME),
    };
    // Its answer is in, or its time is up: nothing of it is left running here. (The unit a real checker
    // runs in is systemd's, with a time limit of its own; the next check stops one left over.)
    if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
    let _ = child.wait();
    said
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_is_forty_lowercase_hex_digits() {
        assert!(tree_id("0123456789abcdef0123456789abcdef01234567"));
        for bad in ["", "HEAD", "0123456789ABCDEF0123456789abcdef01234567", "0123456789abcdef0123456789abcdef0123456", "0123456789abcdef0123456789abcdef012345678", "$(id)0123456789abcdef0123456789abcdef012", "0123456789abcdef0123456789abcdef0123456\n", "--0123456789abcdef0123456789abcdef012345"] {
            assert!(!tree_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn only_the_exact_line_is_an_answer() {
        assert_eq!(parse(b"{\"ok\":true,\"warnings\":[]}\n"), Said::Valid(vec![]));
        assert_eq!(parse(b"{\"ok\":true,\"warnings\":[\"w\"]}"), Said::Valid(vec!["w".into()]));
        assert_eq!(parse(b"{\"ok\":false,\"why\":\"\\\"x\\\" isn't a package.\"}\n"), Said::NotValid("\"x\" isn't a package.".into()));
        for none in [
            &b""[..],
            b"\n",
            b"ok\n",
            b"true\n",
            b"{}\n",
            b"{\"ok\":true}\n",
            b"{\"ok\":false}\n",
            b"{\"ok\":false,\"why\":\"\"}\n",
            b"{\"ok\":false,\"why\":7}\n",
            b"{\"ok\":\"false\",\"why\":\"x\"}\n",
            b"{\"ok\":true,\"why\":\"x\"}\n",
            b"{\"ok\":false,\"warnings\":[]}\n",
            b"{\"ok\":false,\"why\":\"x\",\"warnings\":[]}\n",
            b"{\"ok\":true,\"warnings\":[],\"extra\":1}\n",
            b"{\"ok\":true,\"warnings\":[1]}\n",
            b"{\"ok\":true,\"warnings\":[]}\n{\"ok\":false,\"why\":\"x\"}\n",
            b"warning: something\n{\"ok\":false,\"why\":\"x\"}\n",
            b"{\"ok\":false,\"why\":\"x\"}\n\n",
            b"{\"ok\":false,\"why\":\"\xff\"}\n",
        ] {
            assert_eq!(parse(none), Said::Nothing(NO_ANSWER), "{:?}", String::from_utf8_lossy(none));
        }
    }

    #[test]
    fn what_is_kept_is_short_and_one_line() {
        // Escapes and line breaks in the reason: one line, nothing a terminal or the ledger's reader obeys.
        let Said::NotValid(why) = parse(br#"{"ok":false,"why":"first\nsecond\u001b[31m red\ttab\r\u0000"}"#) else { panic!("not read") };
        assert_eq!(why, "first?second?[31m red?tab??");
        let long = format!(r#"{{"ok":false,"why":"{}"}}"#, "x".repeat(5000));
        let Said::NotValid(why) = parse(long.as_bytes()) else { panic!("not read") };
        assert_eq!(why.chars().count(), TEXT_MAX);
        let many = format!(r#"{{"ok":true,"warnings":[{}]}}"#, (0..50).map(|i| format!("\"w{i}\\nmore {}\"", "y".repeat(1000))).collect::<Vec<_>>().join(","));
        let Said::Valid(w) = parse(many.as_bytes()) else { panic!("not read") };
        assert_eq!(w.len(), WARNINGS_MAX);
        assert!(w.iter().all(|w| w.chars().count() == TEXT_MAX && !w.contains('\n')), "{w:?}");
    }

    fn soon(secs: u64) -> Instant {
        Instant::now() + Duration::from_secs(secs)
    }

    #[test]
    fn the_exit_status_says_nothing() {
        // A checker that answers and fails, and one that succeeds and says nothing.
        assert_eq!(run(r#"echo '{"ok":true,"warnings":[]}'; exit 1"#, soon(20)), Said::Valid(vec![]));
        assert_eq!(run(r#"echo '{"ok":false,"why":"no"}'; exit 0"#, soon(20)), Said::NotValid("no".into()));
        assert_eq!(run("exit 0", soon(20)), Said::Nothing(NO_ANSWER));
        assert_eq!(run("exit 1", soon(20)), Said::Nothing(NO_ANSWER));
        assert_eq!(run("kill -9 $$", soon(20)), Said::Nothing(NO_ANSWER));
        // Its errors are not its answer, and not ours.
        assert_eq!(run(r#"echo '{"ok":false,"why":"no"}' >&2"#, soon(20)), Said::Nothing(NO_ANSWER));
    }

    #[test]
    fn a_flood_or_a_checker_that_never_ends_is_no_answer_in_bounded_time() {
        let started = Instant::now();
        // More than is read, without end: stopped as soon as it is more.
        assert_eq!(run("while :; do echo xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; done", soon(20)), Said::Nothing(NO_ANSWER));
        // A valid line and then a flood is a flood.
        assert_eq!(run(r#"echo '{"ok":true,"warnings":[]}'; while :; do echo xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; done"#, soon(20)), Said::Nothing(NO_ANSWER));
        assert!(started.elapsed() < Duration::from_secs(15), "the flood was read to its end: {:?}", started.elapsed());
        // Never ends, says nothing: given up at the time, and stopped — with what it started.
        let dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("still-running");
        let started = Instant::now();
        let never = format!("(sleep 3; touch {}) & sleep 600", mark.display());
        assert_eq!(run(&never, soon(1)), Said::Nothing(NOT_IN_TIME));
        assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
        // An answer that comes, with something left running that holds its output open.
        let started = Instant::now();
        assert_eq!(run(r#"echo '{"ok":true,"warnings":[]}'; sleep 600"#, soon(1)), Said::Nothing(NOT_IN_TIME));
        assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
        std::thread::sleep(Duration::from_secs(4));
        assert!(!mark.exists(), "what the checker started outlived it");
        // A checker that is not there: the shell says so on its errors, which are no answer.
        assert_eq!(run("/nonexistent/checker", soon(20)), Said::Nothing(NO_ANSWER));
    }
}
