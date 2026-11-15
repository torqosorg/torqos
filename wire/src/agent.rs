//! A person's agent service (ADR-051 3, ADR-055 3): `torq-agent@<person>.service` on its socket
//! `/run/torq-agent/<person>.sock`, one JSON line at a time each way. A client (`torq ask`, the panel's
//! applet, the agent's window) sends a `Request`; the service answers with `Event`s, as they come. The
//! service is the bridge to the person's engine (their coding tool, `torq agent engine`): it hands the
//! question over and passes on what the engine says and does, then says `done`. Every request ends
//! with a last frame, on every way it can go: `done` (an ask; and any refusal or error, exit 1),
//! `status` (status, stop) or `forgotten` (forget).
//!
//! Who is asking is never in a request: the service takes it from the kernel (SO_PEERCRED) and answers
//! only the person it serves.
//!
//! `Task` and `Turn` are the old loop's (`torq-doctor --propose --task`, which wrote the same events on its
//! output): nothing starts it that way any more, and they go with it (docs/HARNESS-AS-A-SYSTEM.md, phase 2).

use serde::{Deserialize, Serialize};

/// The directory of every person's agent socket (root's; each socket is its person's, 0600).
pub const SOCKET_DIR: &str = "/run/torq-agent";

/// Where `person`'s agent listens.
pub fn socket(person: &str) -> String {
    format!("{SOCKET_DIR}/{person}.sock")
}

/// The longest line either side reads: a request, an event (an answer is at most this long).
pub const MAX_LINE: usize = 1 << 20;

/// What a client asks. An unknown kind or field is refused, not ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Request {
    /// A question or a task, with what the person attached to it (an alarm, a failed commit): the
    /// attached text goes to the engine with the question, marked as attached: something to read.
    Ask {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<String>,
    },
    /// Stop the question being answered (the engine's whole process group, and everything of it). The
    /// conversation is kept: the next question continues it.
    Stop,
    /// Whether a question is being answered.
    Status,
    /// End the conversation: the next question starts a new one.
    Forget,
}

/// What the service says. Strict, as `Request` is: a kind or a field one
/// side does not know is refused, never guessed at. Both sides are one
/// system's build: a switch that changes either restarts the service (its unit names the new programs), so a
/// client and the service it reaches differ only for a moment, and then say so rather than misread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Event {
    /// A part of the answer, cleaned of escapes and invisible characters.
    Say { text: String },
    /// Something the engine did on the way, in words a person reads ("ran a command", "read a file"); a tool
    /// TorqOS has no words for, by its own name.
    Called { tool: String },
    /// A proposal the engine made through TorqOS's tools: the broker's number for it.
    Proposed { seq: u64 },
    /// What the answer proposes is marked tainted at the broker, and why (a source's word: `coding-tool`).
    Tainted { why: String },
    /// Something failed; what, in words.
    Error { text: String },
    /// The ask is over: 0 when it was answered, 137 when it was stopped (128 + the signal that ended the engine),
    /// 1 when it failed or was refused (the error before it says why). Every ask ends with it.
    Done { exit: i32 },
    /// The answer to `status` (and to `stop`, after it): whether a question is being answered.
    Status {
        running: bool,
        /// How many questions this conversation has had (shown on the Agent page).
        #[serde(default)]
        turns: usize,
        /// Which conversation this is: a number that changes whenever the conversation is forgotten, so a window
        /// that last saw another one knows its view is old. It says nothing of what was said; absent (0) from an
        /// older agent, which cannot tell.
        #[serde(default)]
        conversation: u64,
    },
    /// The conversation is forgotten.
    Forgotten,
}

/// One turn the agent remembers: what the person asked, and what it answered — kept only when the
/// answer came from a session that read nothing untrusted (else `None`: a tainted answer is never
/// carried into a later session as if it were the agent's own).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Turn {
    pub question: String,
    #[serde(default)]
    pub answer: Option<String>,
}

/// What the service hands the harness for one task, on its input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// The conversation's recent turns, oldest first.
    #[serde(default)]
    pub memory: Vec<Turn>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bytes on the socket, pinned: a change to this format is a deliberate one, made on both sides at once.
    #[test]
    fn the_lines_on_the_socket() {
        let ask = Request::Ask { text: "why?".into(), context: None };
        assert_eq!(serde_json::to_string(&ask).unwrap(), r#"{"type":"ask","text":"why?"}"#);
        let with = Request::Ask { text: "this".into(), context: Some("alarm".into()) };
        assert_eq!(serde_json::to_string(&with).unwrap(), r#"{"type":"ask","text":"this","context":"alarm"}"#);
        assert_eq!(serde_json::to_string(&Request::Stop).unwrap(), r#"{"type":"stop"}"#);
        assert_eq!(serde_json::to_string(&Request::Status).unwrap(), r#"{"type":"status"}"#);
        assert_eq!(serde_json::to_string(&Request::Forget).unwrap(), r#"{"type":"forget"}"#);
        for (e, line) in [
            (Event::Say { text: "a\n".into() }, r#"{"type":"say","text":"a\n"}"#),
            (Event::Called { tool: "read_journal".into() }, r#"{"type":"called","tool":"read_journal"}"#),
            (Event::Proposed { seq: 7 }, r#"{"type":"proposed","seq":7}"#),
            (Event::Tainted { why: "context".into() }, r#"{"type":"tainted","why":"context"}"#),
            (Event::Error { text: "no".into() }, r#"{"type":"error","text":"no"}"#),
            (Event::Done { exit: 137 }, r#"{"type":"done","exit":137}"#),
            (Event::Status { running: false, turns: 3, conversation: 5 }, r#"{"type":"status","running":false,"turns":3,"conversation":5}"#),
            (Event::Forgotten, r#"{"type":"forgotten"}"#),
        ] {
            assert_eq!(serde_json::to_string(&e).unwrap(), line);
            assert_eq!(serde_json::from_str::<Event>(line).unwrap(), e);
        }
        let t = Task { question: "q".into(), context: None, memory: vec![Turn { question: "p".into(), answer: None }] };
        assert_eq!(serde_json::to_string(&t).unwrap(), r#"{"question":"q","memory":[{"question":"p","answer":null}]}"#);
    }

    #[test]
    fn an_unknown_kind_or_field_is_refused() {
        for bad in [r#"{"type":"apply","seq":3}"#, r#"{"type":"ask","text":"x","as":"bob"}"#, r#"{"text":"x"}"#, r#""stop""#, r#"{"type":"ask"}"#] {
            assert!(serde_json::from_str::<Request>(bad).is_err(), "{bad}");
        }
        assert!(serde_json::from_str::<Task>(r#"{"question":"q","tools":["shell"]}"#).is_err());
    }

    #[test]
    fn where_each_agent_listens() {
        assert_eq!(socket("alice"), "/run/torq-agent/alice.sock");
    }
}
