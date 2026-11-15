//! What the panel's applet and the agent's window show of a conversation (ADR-051 4): each question,
//! its answer as it comes, the tools it called, whether its session read untrusted text, what it
//! proposed, and how it ended. Kept here, tested, so the two programs only draw it. Everything the agent
//! says is cleaned again (nothing in it can draw on a screen), and bounded.

use super::client::{Status, Unreachable};
use torq_wire::agent::Event;

/// The most of an answer kept (the harness cuts its own at 64 000 characters).
pub const MAX_ANSWER: usize = 64_000;
/// The most turns a window keeps on screen.
pub const MAX_TURNS: usize = 50;
/// The most tools, sources and errors kept of one turn.
const MAX_ITEMS: usize = 64;

/// How a turn ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Ended {
    /// Still being answered.
    Working,
    Answered,
    /// Stopped, by the person (or the service).
    Stopped,
    /// It failed: the task's code, or no answer at all.
    Failed,
}

/// One question and what came of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub question: String,
    pub answer: String,
    /// What it did on the way, in order: each tool it used, in words ("ran a command").
    pub called: Vec<String>,
    /// Why its session is tainted: each source of untrusted text it read (none: untainted).
    pub tainted: Vec<String>,
    /// The proposals it made, by number.
    pub proposed: Vec<u64>,
    pub errors: Vec<String>,
    pub ended: Ended,
}

/// A text the agent said, as it is shown: each line cleaned, at most `max` characters.
pub fn shown(text: &str, max: usize) -> String {
    let lines: Vec<String> = text.lines().map(crate::history::clean).collect();
    let all = lines.join("\n");
    if all.chars().count() <= max {
        all
    } else {
        format!("{}…", all.chars().take(max.saturating_sub(1)).collect::<String>())
    }
}

fn push_bounded<T>(v: &mut Vec<T>, x: T) {
    if v.len() < MAX_ITEMS {
        v.push(x);
    }
}

/// The turns of this window's conversation, oldest first.
#[derive(Debug, Default, Clone)]
pub struct Conversation {
    turns: std::collections::VecDeque<Turn>,
}

impl Conversation {
    /// A question asked: a turn, working.
    pub fn asked(&mut self, question: &str) {
        if self.turns.len() >= MAX_TURNS {
            self.turns.pop_front();
        }
        self.turns.push_back(Turn {
            question: shown(question, MAX_ANSWER),
            answer: String::new(),
            called: vec![],
            tainted: vec![],
            proposed: vec![],
            errors: vec![],
            ended: Ended::Working,
        });
    }

    /// What the agent said of the turn being answered.
    pub fn heard(&mut self, e: Event) {
        let Some(t) = self.turns.back_mut().filter(|t| t.ended == Ended::Working) else { return };
        match e {
            Event::Say { text } => {
                if !t.answer.is_empty() {
                    t.answer.push('\n');
                }
                t.answer.push_str(&text);
                t.answer = shown(&t.answer, MAX_ANSWER);
            }
            Event::Called { tool } => push_bounded(&mut t.called, crate::history::one_line(&tool, 64)),
            Event::Tainted { why } => push_bounded(&mut t.tainted, crate::history::one_line(&why, 64)),
            Event::Proposed { seq } => push_bounded(&mut t.proposed, seq),
            Event::Error { text } => push_bounded(&mut t.errors, shown(&text, 2000)),
            // Killed by `stop` (its signal's code, 128 + 9) is stopped, not failed.
            Event::Done { exit } => {
                t.ended = match exit {
                    0 if t.errors.is_empty() => Ended::Answered,
                    137 => Ended::Stopped,
                    _ => Ended::Failed,
                }
            }
            Event::Status { .. } | Event::Forgotten => {}
        }
    }

    /// The turn being answered could not be: why (the service gone, silent, refused).
    pub fn lost(&mut self, why: &str) {
        if let Some(t) = self.turns.back_mut().filter(|t| t.ended == Ended::Working) {
            push_bounded(&mut t.errors, shown(why, 2000));
            t.ended = Ended::Failed;
        }
    }

    /// Whether a question of this window's is being answered.
    pub fn working(&self) -> bool {
        self.turns.back().is_some_and(|t| t.ended == Ended::Working)
    }

    pub fn turns(&self) -> impl DoubleEndedIterator<Item = &Turn> {
        self.turns.iter()
    }

    pub fn last(&self) -> Option<&Turn> {
        self.turns.back()
    }

    /// Every proposal made in this conversation, newest first.
    pub fn proposals(&self) -> Vec<u64> {
        self.turns.iter().rev().flat_map(|t| t.proposed.iter().rev().copied()).collect()
    }

    /// The window's own record emptied, as the conversation is deleted by `forget`.
    pub fn clear(&mut self) {
        self.turns.clear();
    }
}

impl Turn {
    /// How the turn stands, in words.
    pub fn state(&self) -> String {
        let mut s = match self.ended {
            Ended::Working => "Working…".to_string(),
            Ended::Answered => "Answered".to_string(),
            Ended::Stopped => "Stopped".to_string(),
            Ended::Failed => "Did not finish".to_string(),
        };
        // What it did on the way, each kind once, in the order it first did it: "ran a command (3 times), read a file".
        let mut did: Vec<(&str, usize)> = Vec::new();
        for c in &self.called {
            match did.iter_mut().find(|(name, _)| *name == c.as_str()) {
                Some((_, n)) => *n += 1,
                None => did.push((c, 1)),
            }
        }
        if !did.is_empty() {
            let said: Vec<String> = did.iter().map(|(name, n)| if *n == 1 { name.to_string() } else { format!("{name} ({n} times)") }).collect();
            s.push_str(&format!(" · {}", said.join(", ")));
        }
        s
    }

    /// Said where the turn proposed something (the broker marks every proposal of a coding tool's, whatever
    /// it read; its own word for that is not shown): what that means to the person.

    pub fn taint(&self) -> Option<String> {
        (!self.tainted.is_empty()).then(|| super::ALWAYS_ASKED.to_string())
    }
}

/// How the agent's mark stands in the panel: quiet when idle, the accent while it works, the warning when
/// something is wrong.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mood {
    Quiet,
    Working,
    Trouble,
}

/// The agent at a glance — its one line, and its mark — from what its service said (`None`: not running,
/// so idle and not asked, since asking would start it) and from this program's own last turn.
pub fn glance(reach: &Result<Option<Status>, Unreachable>, here: Option<&Turn>) -> (String, Mood) {
    if here.is_some_and(|t| t.ended == Ended::Working) {
        return ("Working…".into(), Mood::Working);
    }
    let failed = here.is_some_and(|t| t.ended == Ended::Failed);
    match reach {
        Ok(Some(s)) if s.running => ("Working…".into(), Mood::Working),
        Ok(Some(s)) => {
            let kept = match s.turns {
                0 => "no conversation yet".to_string(),
                1 => "a conversation of 1 question".to_string(),
                n => format!("a conversation of {n} questions"),
            };
            (format!("Idle · {kept}"), if failed { Mood::Trouble } else { Mood::Quiet })
        }
        Ok(None) => ("Idle".into(), if failed { Mood::Trouble } else { Mood::Quiet }),
        Err(e) if e.not_set_up() => (e.line(), Mood::Quiet),
        Err(e) => (e.line(), Mood::Trouble),
    }
}

/// What became of a proposal, by the ledger.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fate {
    /// Waiting for a person (as TorqOS Settings' Proposals page lists it).
    Waiting,
    Applied,
    /// A person declined it (ADR-051, amended): never applied.
    Declined,
    /// Its apply failed: it waits again, for a person to apply it again or decline it (ADR-008).
    Failed,
    /// The ledger could not be read to look it up (said, rather than "looking it up…" for good).
    /// Looked up again at the next look.
    Unread,
    /// No proposal of this person's agent has that number: no Review for it.
    NotYours,
}

/// Each of `seqs`' fate in the ledger's `entries`, where `agent` is the person's agent account: a number
/// that is not a proposal of that agent's is `NotYours`, whatever the service said.
pub fn fates(entries: &[crate::history::Entry], agent: &str, seqs: &[u64]) -> Vec<(u64, Fate)> {
    let waiting = crate::history::waiting_proposals(entries);
    seqs.iter()
        .map(|&seq| {
            let mine = entries.iter().any(|e| e.seq == seq && e.kind == "proposal" && e.data["agent"].as_str() == Some(agent));
            let ended = |kind: &str| entries.iter().any(|e| e.kind == kind && e.data["proposal"].as_u64() == Some(seq));
            let fate = if !mine {
                Fate::NotYours
            } else if waiting.iter().any(|e| e.seq == seq) && crate::history::failed_once(entries, seq) {
                Fate::Failed
            } else if waiting.iter().any(|e| e.seq == seq) {
                Fate::Waiting
            } else if ended("declined") {
                Fate::Declined
            } else if ended("applied") {
                Fate::Applied
            } else {
                Fate::Failed
            };
            (seq, fate)
        })
        .collect()
}

/// A proposal as a turn shows it: its words, and whether it has a Review (only one of the person's own
/// agent's, waiting).
#[derive(Debug, Clone, PartialEq)]
pub struct ProposalRow {
    pub seq: u64,
    pub text: String,
    pub review: bool,
}

impl Turn {
    /// The proposals this turn made, each with its fate (not yet looked up: said so, no Review).

    pub fn proposal_rows(&self, fates: &[(u64, Fate)]) -> Vec<ProposalRow> {

        self.proposed
            .iter()
            .map(|&seq| {
                let fate = fates.iter().find(|(s, _)| *s == seq).map(|(_, f)| *f);
                let said = match fate {
                    None => "looking it up…",
                    Some(Fate::Waiting) => "waiting for you to apply or refuse it",
                    Some(Fate::Applied) => "applied",
                    Some(Fate::Declined) => "declined",
                    Some(Fate::Failed) => "its apply failed once — apply it again, or decline it",
                    Some(Fate::NotYours) => "not your agent's proposal",
                    Some(Fate::Unread) => "what became of it could not be read (the ledger): looked up again shortly",
                };
                ProposalRow { seq, text: format!("Proposed #{seq}: {said}"), review: matches!(fate, Some(Fate::Waiting | Fate::Failed)) }
            })
            .collect()
    }
}

/// Which conversation of the agent's a window last saw, and how long it was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seen {
    conversation: u64,
    turns: usize,
}

impl Seen {
    pub fn of(st: &Status) -> Seen {
        Seen { conversation: st.conversation, turns: st.turns }
    }

    /// The window's own Forget: it knows the conversation is empty (the number it will see next is not known yet,
    /// so the next look settles it without a reset: see `forgotten`).
    pub fn empty() -> Seen {
        Seen { conversation: 0, turns: 0 }
    }
}

/// Whether the agent's conversation is another one than `before` saw (a Forget, from anywhere). By the
/// conversation's number when both sides name one; else, for an older agent that does not, by the memory
/// shrinking, which it never does otherwise (it keeps its last turns, and drops the oldest as it adds one).
pub fn forgotten(before: Option<Seen>, now: Seen) -> bool {
    before.is_some_and(|b| if b.conversation != 0 && now.conversation != 0 { b.conversation != now.conversation } else { now.turns < b.turns })
}

/// What a window does at a look at `st`: whether its view of the conversation is of an older one and must
/// be reset. `seen` is updated, but not while the window's own task runs: a Forget from elsewhere then
/// would be taken as the count and missed, and no reset is done mid-task (it would cut the answer being
/// heard).
pub fn at_look(seen: &mut Option<Seen>, st: &Status, working: bool) -> bool {
    if working {
        return false;
    }
    let reset = forgotten(*seen, Seen::of(st));
    *seen = Some(Seen::of(st));
    reset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_as_it_comes() {
        let mut c = Conversation::default();
        assert!(!c.working());
        c.asked("why\u{1b}[2J?");
        assert!(c.working());
        c.heard(Event::Called { tool: "read_journal".into() });
        c.heard(Event::Tainted { why: "read_journal".into() });
        c.heard(Event::Say { text: "line one\r\nrewrite\u{202e}".into() });
        c.heard(Event::Proposed { seq: 12 });
        c.heard(Event::Done { exit: 0 });
        let t = c.last().unwrap();
        assert!(!c.working() && t.ended == Ended::Answered);
        assert!(!t.question.contains('\u{1b}') && !t.answer.contains(['\r', '\u{202e}']), "{t:?}");
        assert!(t.answer.starts_with("line one\nrewrite"), "{:?}", t.answer);
        assert_eq!(t.state(), "Answered · read_journal");
        assert_eq!(t.taint().unwrap(), "Your agent's proposals always wait for you to read and approve them.", "the broker's word for it is not shown");
        assert_eq!(c.proposals(), vec![12]);
        // What it did, as the engine's steps are said: each kind once, counted.
        let mut d = Conversation::default();
        d.asked("fix it");
        for tool in ["ran a command", "read a file", "ran a command", "changed a file", "ran a command"] {
            d.heard(Event::Called { tool: tool.into() });
        }
        assert_eq!(d.last().unwrap().state(), "Working… · ran a command (3 times), read a file, changed a file");
        // Nothing more lands on a turn that is over.
        c.heard(Event::Say { text: "late".into() });
        assert!(!c.last().unwrap().answer.contains("late"));
    }

    #[test]
    fn how_a_turn_ends() {
        let mut c = Conversation::default();
        let end = |c: &mut Conversation, events: Vec<Event>| {
            c.asked("q");
            events.into_iter().for_each(|e| c.heard(e));
            c.last().unwrap().ended.clone()
        };
        assert_eq!(end(&mut c, vec![Event::Done { exit: 137 }]), Ended::Stopped);
        assert_eq!(end(&mut c, vec![Event::Done { exit: 1 }]), Ended::Failed);
        assert_eq!(end(&mut c, vec![Event::Error { text: "a task is running already".into() }, Event::Done { exit: 0 }]), Ended::Failed);
        c.asked("q");
        c.lost("your agent did not answer in time");
        assert_eq!(c.last().unwrap().ended, Ended::Failed);
        assert_eq!(c.last().unwrap().errors, ["your agent did not answer in time"]);
        assert!(c.last().unwrap().taint().is_none());
    }

    #[test]
    fn bounded() {
        let mut c = Conversation::default();
        for i in 0..MAX_TURNS + 5 {
            c.asked(&format!("q{i}"));
            c.heard(Event::Proposed { seq: i as u64 });
            c.heard(Event::Done { exit: 0 });
        }
        assert_eq!(c.turns().count(), MAX_TURNS);
        assert_eq!(c.turns().next().unwrap().question, "q5", "the oldest go first");
        assert_eq!(c.proposals()[0], MAX_TURNS as u64 + 4, "newest first");
        c.asked("long");
        c.heard(Event::Say { text: "x".repeat(MAX_ANSWER) });
        c.heard(Event::Say { text: "more".into() });
        assert_eq!(c.last().unwrap().answer.chars().count(), MAX_ANSWER);
        for _ in 0..200 {
            c.heard(Event::Called { tool: "t".into() });
        }
        assert_eq!(c.last().unwrap().called.len(), MAX_ITEMS);
        c.clear();
        assert_eq!(c.turns().count(), 0);
    }

    /// The panel's line and mark: idle quiet, working in the accent, trouble in the warning colour; an agent
    /// not set up is said, quietly.
    #[test]
    fn at_a_glance() {
        let idle = |turns| Ok(Some(Status { running: false, turns, conversation: 0 }));
        assert_eq!(glance(&idle(0), None), ("Idle · no conversation yet".into(), Mood::Quiet));
        assert_eq!(glance(&idle(3), None), ("Idle · a conversation of 3 questions".into(), Mood::Quiet));
        assert_eq!(glance(&Ok(None), None), ("Idle".into(), Mood::Quiet), "not running: not asked");
        assert_eq!(glance(&Ok(Some(Status { running: true, turns: 1, conversation: 0 })), None).1, Mood::Working, "a task from anywhere: working");
        assert_eq!(glance(&Err(Unreachable::NoAgent("torq-agent-me".into())), None), ("The agent is not set up for you".into(), Mood::Quiet));
        assert_eq!(glance(&Err(Unreachable::Silent("x".into())), None).1, Mood::Trouble);
        let mut c = Conversation::default();
        c.asked("q");
        assert_eq!(glance(&idle(0), c.last()), ("Working…".into(), Mood::Working), "its own question, before the service says so");
        c.lost("gone");
        assert_eq!(glance(&idle(0), c.last()).1, Mood::Trouble, "its last question failed");
    }

    fn entry(seq: u64, kind: &str, data: serde_json::Value) -> crate::history::Entry {
        crate::history::Entry { seq, ts: String::new(), kind: kind.into(), tier: String::new(), actor: "agent".into(), approved_by: None, tainted: false, reason: String::new(), data }
    }

    /// A proposal's row: its fate from the ledger, Review only for one of the person's own agent's that
    /// waits.
    #[test]
    fn proposals_as_a_turn_shows_them() {
        use serde_json::json;
        let entries = vec![
            entry(1, "proposal", json!({"agent": "torq-agent-me"})),
            entry(2, "proposal", json!({"agent": "torq-agent-me"})),
            entry(3, "applied", json!({"proposal": 2})),
            entry(4, "proposal", json!({"agent": "torq-agent-bob"})),
            entry(5, "proposal", json!({"agent": "torq-agent-me"})),
            entry(6, "apply_failed", json!({"proposal": 5})),
            entry(7, "applied", json!({})),
            entry(10, "proposal", json!({"agent": "torq-agent-me"})),
            entry(11, "declined", json!({"proposal": 10})),
        ];
        let f = fates(&entries, "torq-agent-me", &[1, 2, 4, 5, 7, 10, 99]);
        assert_eq!(f, vec![(1, Fate::Waiting), (2, Fate::Applied), (4, Fate::NotYours), (5, Fate::Failed), (7, Fate::NotYours), (10, Fate::Declined), (99, Fate::NotYours)]);
        let mut c = Conversation::default();
        c.asked("q");
        for seq in [1, 2, 4, 8, 10] {
            c.heard(Event::Proposed { seq });
        }
        let rows = c.last().unwrap().proposal_rows(&f);
        assert_eq!(rows.iter().map(|r| (r.seq, r.review)).collect::<Vec<_>>(), [(1, true), (2, false), (4, false), (8, false), (10, false)]);
        assert_eq!(rows[0].text, "Proposed #1: waiting for you to apply or refuse it");
        assert_eq!(rows[3].text, "Proposed #8: looking it up…");
        assert_eq!(rows[4].text, "Proposed #10: declined");
        c.heard(Event::Tainted { why: "read_journal".into() });
        let rows = c.last().unwrap().proposal_rows(&f);
        assert!(rows.iter().all(|r| !r.text.contains("taint")), "the mark is the turn's one sentence, not a word on each row: {rows:?}");
    }

    #[test]
    fn forgotten_elsewhere() {
        let seen = |conversation, turns| Seen { conversation, turns };
        // By the number: another conversation, whatever its length (a Forget and then as many turns again is caught).
        assert!(forgotten(Some(seen(1, 3)), seen(2, 3)) && forgotten(Some(seen(1, 3)), seen(2, 0)));
        assert!(!forgotten(Some(seen(2, 3)), seen(2, 4)) && !forgotten(Some(seen(2, 8)), seen(2, 8)));
        assert!(!forgotten(None, seen(2, 0)), "never seen: nothing to say");
        // An older agent names none (0): by the memory shrinking, as before.
        assert!(forgotten(Some(seen(0, 3)), seen(0, 0)) && forgotten(Some(seen(0, 8)), seen(0, 1)));
        assert!(!forgotten(Some(seen(0, 8)), seen(0, 8)) && !forgotten(Some(seen(0, 2)), seen(0, 3)));
        // An agent updated under a window: its number is not a Forget.
        assert!(!forgotten(Some(seen(0, 3)), seen(5, 3)) && !forgotten(Some(seen(5, 3)), seen(0, 3)));
        assert!(!forgotten(Some(Seen::empty()), seen(5, 0)), "the window's own Forget settles at the next look");
    }

    #[test]
    fn a_look_resets_a_view_of_an_older_conversation() {
        let st = |conversation, turns| Status { running: false, turns, conversation };
        let mut seen = None;
        assert!(!at_look(&mut seen, &st(1, 2), false), "the first look: nothing to reset");
        assert!(!at_look(&mut seen, &st(1, 3), false));
        assert!(at_look(&mut seen, &st(2, 3), false), "forgotten elsewhere, and as long again");
        assert!(!at_look(&mut seen, &st(2, 3), false), "said once");
        // While the window's own task runs, nothing is reset and nothing noted: the Forget is seen after it.
        assert!(!at_look(&mut seen, &st(3, 0), true) && seen == Some(Seen { conversation: 2, turns: 3 }));
        assert!(at_look(&mut seen, &st(3, 0), false));
        // An older agent: by the count.
        let mut old = None;
        at_look(&mut old, &st(0, 4), false);
        assert!(at_look(&mut old, &st(0, 0), false));
    }
}
