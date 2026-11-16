//! The record of what each person's agent asked its model, and where on the network it went (ADR-057).
//! The model proxy (`torq-model-proxy`) writes it; torqcli and TorqOS Settings read it, as the person. Both
//! use this module, so a word one of them knows they both know.
//!
//! Where: `/var/lib/torq-model-proxy/<person>/<day>.<part>.jsonl` (`DIR`), one line of JSON for each thing
//! recorded (`Entry`), oldest first. The day is the date in UTC of the moment the line was written, the part
//! a number from 000 that goes up when a file is full (`MAX_FILE`); sorted by name the files are in order.
//! The folder is the proxy's; the person reads it (an access rule for them alone), no one else does but root,
//! and no agent's account reads or writes it. It is not in the broker's ledger and nothing chains its lines:
//! root, and the proxy's own account, can change it.
//!
//! It never holds a credential: no header's value is kept but the three the engine names its conversation and
//! its helpers by. It does hold what the agent asked and read, cut to `MAX_TEXT` each: as private as that is.

use serde::{Deserialize, Serialize};

/// Where the record is kept: a folder for each person in it.
pub const DIR: &str = "/var/lib/torq-model-proxy";
/// The shape of a line, as `Entry::v` says it. A reader that meets a larger number shows what it knows.
pub const VERSION: u32 = 1;

/// The most of one text that is kept (a question, what a tool returned, an answer's text, a tool call's
/// input), in bytes. A longer one is cut here, and its line says how long it was (`cut_from`).
pub const MAX_TEXT: usize = 8 * 1024;
/// The most things kept from one request's new part, and from one answer. More are counted (`left_out`).
pub const MAX_ITEMS: usize = 48;
/// The most of a request's body the proxy reads to find what was new in it, in bytes. A larger request is
/// passed on whole all the same; its line says it was not read (`unread`).
pub const MAX_READ_REQUEST: usize = 16 * 1024 * 1024;
/// The most of an answer the proxy reads, in bytes as the model wrote them (uncompressed). The rest is passed on
/// all the same; the line says so (`unread`).
pub const MAX_READ_ANSWER: usize = 8 * 1024 * 1024;
/// How large one file of the record grows before the next part is begun, in bytes.
pub const MAX_FILE: u64 = 8 * 1024 * 1024;
/// How much is kept for each person, in bytes. Past it the oldest files are removed, and a line says which
/// (`Line::Removed`).
pub const MAX_PERSON: u64 = 256 * 1024 * 1024;
/// The most of a name the engine gives (its session, its helpers' ids, a model, a path), in bytes.
pub const MAX_NAME: usize = 200;

/// One line of the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// `VERSION` when it was written.
    pub v: u32,
    #[serde(flatten)]
    pub line: Line,
}

/// What a line records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Line {
    /// One request the agent made of its model, and the answer.
    Request(Request),
    /// One site the agent asked to reach, allowed or refused.
    Connect(Connect),
    /// An old file of the record removed to keep within `MAX_PERSON`.
    Removed(Removed),
    /// Lines that could not be written (a full disk): how many. While the record cannot be written nothing is
    /// passed on, so these are the requests that were under way when it broke.
    Lost(Lost),
}

/// A request to the model, and its answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// When the request arrived: `2026-11-16T09:00:00Z`.
    pub time: String,
    /// How long it took until the answer ended, in milliseconds.
    pub ms: u64,
    /// The agent's account, by the kernel's word, and the person it serves.
    pub agent: String,
    pub person: String,
    /// The engine's own name for the conversation (`x-claude-code-session-id`), if it gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Which helper of the engine's asked (`x-claude-code-agent-id`) and which started it
    /// (`x-claude-code-parent-agent-id`), if it said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    /// What was asked of the model's service: `POST`, `/v1/messages` (never the part after a `?`).
    pub method: String,
    pub path: String,
    /// The model named in the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// How it ended.
    pub outcome: Outcome,
    /// The service's answer as a number (200 is "answered"), where there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// The bytes of the request's body and of the answer's, as they passed.
    pub sent: u64,
    pub received: u64,
    /// The tokens the service counted, where it said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Tokens>,
    /// Why the model stopped (`end_turn`, `tool_use`, …), where it said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<String>,
    /// The service's own words for an error, where the answer was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// What was new in the conversation in this request: the parts of each message after the model's last one
    /// (the person's question, or each tool's result; a message of the engine's own by its kind only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub new: Vec<Item>,
    /// What the model answered: its text, and each tool it called.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answer: Vec<Item>,
    /// How many things beyond `MAX_ITEMS` were not kept, of the new part and of the answer.
    #[serde(default, skip_serializing_if = "LeftOut::none")]
    pub left_out: LeftOut,
    /// Each part that was passed on but not read for the record, and why, in plain words.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unread: Vec<String>,
}

/// How a request ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// The whole answer passed.
    #[default]
    Complete,
    /// The answer stopped before its end: the engine went away, or the service did.
    Interrupted,
    /// The model's service was not reached at all.
    NotReached,
}

/// The tokens of one request, as the service counts them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    /// Read by the model, not from its cache.
    pub input: u64,
    /// Read from the service's cache, and written to it.
    pub cache_read: u64,
    pub cache_write: u64,
    /// Written by the model.
    pub output: u64,
}

/// How many things were not kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeftOut {
    #[serde(default)]
    pub new: u64,
    #[serde(default)]
    pub answer: u64,
}

impl LeftOut {
    pub fn none(&self) -> bool {
        self.new == 0 && self.answer == 0
    }
}

/// One thing in a request's new part or in an answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Item {
    /// Words: in a request the person's question (or what the engine added to it), in an answer the model's.
    Text {
        text: String,
        /// How long the text was before it was cut to `MAX_TEXT`, in bytes; absent where it is whole.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cut_from: Option<u64>,
    },
    /// What a tool returned to the model.
    ToolResult {
        /// The tool's name, where the request still held the call it answers.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool: Option<String>,
        text: String,
        /// Whether the tool reported a failure.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cut_from: Option<u64>,
    },
    /// A tool the model called, and what it gave it (JSON, as text).
    ToolCall {
        tool: String,
        input: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cut_from: Option<u64>,
    },
    /// Something else (a picture, the model's private reasoning): its kind by the service's name, not its content.
    Other { what: String },
}

/// A site the agent asked to reach through the proxy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connect {
    pub time: String,
    pub agent: String,
    pub person: String,
    /// The site's name as the agent gave it, and the port.
    pub host: String,
    pub port: u16,
    pub allowed: bool,
    pub why: Why,
    /// Whether the site answered, where it was allowed. What then passed is not seen: it is encrypted between
    /// the agent and the site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reached: Option<bool>,
}

/// Why a site was allowed or refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Why {
    /// Allowed: on the owner's list (`torq agent network hosts`).
    Listed,
    /// Allowed: signing in to the engine's service and keeping the sign-in fresh need it.
    SignIn,
    /// Allowed: the owner opened the web to the agents (`torq agent network web`).
    Web,
    /// Refused: not on the list.
    NotListed,
    /// Refused: the model's own service, which is reached through the proxy's record and no other way.
    TheModel,
    /// Refused: not HTTPS (a port other than 443).
    NotHttps,
    /// Refused: the name is an address on this machine, or one on its own network that the owner did not list.
    Address,
    /// Refused: not a site's name at all.
    NotAName,
}

/// An old file of the record, removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Removed {
    pub time: String,
    /// The file's name (`2026-11-16.000.jsonl`).
    pub file: String,
}

/// Lines that could not be written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lost {
    pub time: String,
    pub lines: u64,
}

/// A name the engine gave, safe to keep and to show: printable ASCII without spaces, at most `MAX_NAME`. One
/// that is anything else is not kept at all.
pub fn name(s: &str) -> Option<String> {
    (!s.is_empty() && s.len() <= MAX_NAME && s.bytes().all(|b| b.is_ascii_graphic())).then(|| s.to_string())
}

/// `text` cut to `MAX_TEXT` bytes at a character's edge, and its full length where it was cut.
pub fn cut(text: &str) -> (String, Option<u64>) {
    if text.len() <= MAX_TEXT {
        return (text.to_string(), None);
    }
    let mut end = MAX_TEXT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), Some(text.len() as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bytes of a line, pinned: torqcli reads what the proxy of any earlier day wrote.
    #[test]
    fn a_request_line_is_these_bytes() {
        let e = Entry {
            v: VERSION,
            line: Line::Request(Request {
                time: "2026-11-16T09:00:00Z".into(),
                ms: 1200,
                agent: "torq-agent-alice".into(),
                person: "alice".into(),
                session: Some("s-1".into()),
                method: "POST".into(),
                path: "/v1/messages".into(),
                model: Some("m".into()),
                status: Some(200),
                sent: 10,
                received: 20,
                tokens: Some(Tokens { input: 1, cache_read: 2, cache_write: 3, output: 4 }),
                stop: Some("tool_use".into()),
                new: vec![Item::ToolResult { tool: Some("Bash".into()), text: "ok".into(), error: false, cut_from: None }],
                answer: vec![Item::Text { text: "hi".into(), cut_from: Some(9000) }, Item::ToolCall { tool: "Read".into(), input: "{}".into(), cut_from: None }, Item::Other { what: "thinking".into() }],
                ..Default::default()
            }),
        };
        let s = serde_json::to_string(&e).unwrap();
        assert_eq!(s, r#"{"v":1,"kind":"request","time":"2026-11-16T09:00:00Z","ms":1200,"agent":"torq-agent-alice","person":"alice","session":"s-1","method":"POST","path":"/v1/messages","model":"m","outcome":"complete","status":200,"sent":10,"received":20,"tokens":{"input":1,"cache_read":2,"cache_write":3,"output":4},"stop":"tool_use","new":[{"kind":"tool-result","tool":"Bash","text":"ok"}],"answer":[{"kind":"text","text":"hi","cut_from":9000},{"kind":"tool-call","tool":"Read","input":"{}"},{"kind":"other","what":"thinking"}]}"#);
        assert_eq!(serde_json::from_str::<Entry>(&s).unwrap(), e, "and read back the same");
    }

    #[test]
    fn the_other_lines_are_these_bytes() {
        let c = Entry { v: VERSION, line: Line::Connect(Connect { time: "t".into(), agent: "torq-agent-bob".into(), person: "bob".into(), host: "example.org".into(), port: 443, allowed: false, why: Why::NotListed, reached: None }) };
        let s = serde_json::to_string(&c).unwrap();
        assert_eq!(s, r#"{"v":1,"kind":"connect","time":"t","agent":"torq-agent-bob","person":"bob","host":"example.org","port":443,"allowed":false,"why":"not-listed"}"#);
        assert_eq!(serde_json::from_str::<Entry>(&s).unwrap(), c);
        let r = Entry { v: VERSION, line: Line::Removed(Removed { time: "t".into(), file: "2026-11-01.000.jsonl".into() }) };
        assert_eq!(serde_json::to_string(&r).unwrap(), r#"{"v":1,"kind":"removed","time":"t","file":"2026-11-01.000.jsonl"}"#);
        let l = Entry { v: VERSION, line: Line::Lost(Lost { time: "t".into(), lines: 3 }) };
        assert_eq!(serde_json::to_string(&l).unwrap(), r#"{"v":1,"kind":"lost","time":"t","lines":3}"#);
        assert_eq!(serde_json::from_str::<Entry>(r#"{"v":1,"kind":"lost","time":"t","lines":3}"#).unwrap(), l);
    }

    #[test]
    fn a_text_is_cut_at_a_characters_edge_and_says_so() {
        assert_eq!(cut("short"), ("short".to_string(), None));
        let long = "é".repeat(MAX_TEXT);
        let (kept, from) = cut(&long);
        assert_eq!(from, Some(2 * MAX_TEXT as u64));
        assert!(kept.len() <= MAX_TEXT && kept.chars().all(|c| c == 'é'));
        let odd = format!("a{}", "é".repeat(MAX_TEXT));
        assert_eq!(cut(&odd).0.len(), MAX_TEXT - 1, "never in the middle of a character");
    }

    #[test]
    fn a_name_is_plain_or_not_kept() {
        assert_eq!(name("0fa37e66-5a57").as_deref(), Some("0fa37e66-5a57"));
        assert_eq!(name("two words"), None);
        assert_eq!(name("line\nbreak"), None);
        assert_eq!(name(""), None);
        assert_eq!(name(&"x".repeat(MAX_NAME + 1)), None);
    }
}
