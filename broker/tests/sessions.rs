//! The agent's sessions, held by the broker (ADR-042, amended; session.rs): every one is tainted from its
//! start, every proposal of an agent's waits for a person, and nothing applies one without a person
//! (session trust was removed in phase 2 of ADR-055). Each test fails if its guard is taken out.

#![forbid(unsafe_code)]
#![forbid(dead_code)]

use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use torq_broker::ledger::NewEntry;
use torq_broker::{agent::Limits, policy::Actor, session, Broker};

fn setup(settings: &[&str]) -> (tempfile::TempDir, Broker) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("config");
    fs::create_dir_all(repo.join("users")).unwrap();
    fs::write(repo.join("desktop.nix"), "{ }\n").unwrap();
    let limits = Limits { agents: agents(&["casey"]), settings: settings.iter().map(|s| s.to_string()).collect() };
    let b = by_socket(Broker::new(&repo, dir.path().join("state"), "true").with_agent_limits(&limits).as_agent(Some("torq-agent-casey")), "torq-agent-casey");
    b.init().unwrap();
    (dir, b)
}

/// Each person's agent account (ADR-051 3): `torq-agent-<person>` → `<person>`.
fn agents(people: &[&str]) -> std::collections::BTreeMap<String, String> {
    people.iter().map(|p| (format!("torq-agent-{p}"), p.to_string())).collect()
}

fn diff(dir: &Path, name: &str, path: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, format!("--- /dev/null\n+++ b/{path}\n@@ -0,0 +1 @@\n+{body}\n")).unwrap();
    p
}

fn htop(dir: &Path) -> PathBuf {
    diff(dir, "htop.diff", "users/casey/packages.json", "[\"htop\"]")
}

fn propose(b: &Broker, d: &Path, actor: Actor, tainted: bool, session: Option<&str>) -> anyhow::Result<torq_broker::ledger::Entry> {
    b.propose_in(Some(&b.repo.head().unwrap()), d, "install htop", actor, tainted, session)
}

fn kinds(b: &Broker, kind: &str) -> usize {
    b.ledger.read_all().unwrap().iter().filter(|e| e.body.kind == kind).count()
}

/// What the ledger records of a call that came over the socket from `agent` (its account, as the kernel names it).
fn by_socket(mut b: Broker, agent: &str) -> Broker {
    b.ledger.set_via(Some("socket".into()), Some(agent.into()));
    b
}

/// The same machine's state, as another agent account sees it: a second broker run for `agent`.
fn as_agent(dir: &Path, agent: &str, people: &[&str]) -> Broker {
    by_socket(Broker::new(dir.join("config"), dir.join("state"), "true").with_agent_limits(&Limits { agents: agents(people), settings: vec![] }).as_agent(Some(agent)), agent)
}

#[test]
fn an_agents_proposal_without_a_session_is_tainted() {
    let (dir, b) = setup(&[]);
    let p = propose(&b, &htop(dir.path()), Actor::Agent, false, None).unwrap();
    assert!(p.body.tainted, "no session: tainted, whatever the caller says");
    assert!(b.apply(p.body.seq, "casey", false).is_err(), "a tainted proposal needs --confirm-tainted");
    // A person's proposal is as they say.
    let p = propose(&b, &diff(dir.path(), "p.diff", "users/sam/packages.json", "[\"jq\"]"), Actor::User, false, None).unwrap();
    assert!(!p.body.tainted);
    let p = propose(&b, &diff(dir.path(), "q.diff", "users/sam/packages.json", "[\"tree\"]"), Actor::User, true, None).unwrap();
    assert!(p.body.tainted, "a person's own word for it");
}

/// Phase 2 of ADR-055: every session of an agent account's is tainted from its start, and every proposal of
/// an agent's is, whatever session it names and whatever its caller says. So each waits for a person: an
/// apply without the person's explicit confirmation is refused, and with it, it is that person's change.
#[test]
fn every_session_and_proposal_of_an_agents_is_tainted() {
    let (dir, b) = setup(&[]);
    let s = session::start(&b).unwrap();
    assert!(s.tainted && s.tainted_by == [session::UNSEEN], "{s:?}");
    let p = propose(&b, &htop(dir.path()), Actor::Agent, false, Some(&s.id)).unwrap();
    assert!(p.body.tainted, "{:?}", p.body);
    assert_eq!(p.body.data["session"], json!({ "id": s.id, "started": s.started, "tainted_by": [session::UNSEEN] }));
    assert!(p.body.data.get("resident").is_none(), "no such word any more");
    // Nothing became of it by itself.
    assert_eq!(kinds(&b, "applied"), 0);
    let err = format!("{:#}", b.apply(p.body.seq, "casey", false).unwrap_err());
    assert!(err.contains("--confirm-tainted"), "{err}");
    let a = b.apply(p.body.seq, "casey", true).unwrap();
    assert_eq!((a.body.kind.as_str(), a.body.approved_by.as_deref()), ("applied", Some("casey")));
    assert!(a.body.data.get("trust").is_none());
}

#[test]
fn the_broker_holds_the_record_and_nothing_clears_it() {
    let (dir, b) = setup(&[]);
    let before = b.ledger.read_all().unwrap().len();
    let s = session::start(&b).unwrap();
    // Its record is the broker's, in its state directory, private; the ledger has nothing of it yet.
    use std::os::unix::fs::PermissionsExt;
    let rec = dir.path().join("state/sessions").join(format!("{}.json", s.id));
    assert_eq!(fs::metadata(&rec).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(b.ledger.read_all().unwrap().len(), before, "a session that proposes nothing costs the ledger nothing");
    // What it reports reading is added, each once, for good; a proposal's entry says it.
    session::taint(&b, &s.id, "read_journal").unwrap();
    session::taint(&b, &s.id, "show_ledger").unwrap();
    session::taint(&b, &s.id, "read_journal").unwrap();
    let r = session::read(&b, &s.id).unwrap().unwrap();
    assert!(r.tainted && r.tainted_by == [session::UNSEEN, "read_journal", "show_ledger"], "{r:?}");
    let p = propose(&b, &htop(dir.path()), Actor::Agent, false, Some(&s.id)).unwrap();
    assert!(p.body.tainted);
    assert_eq!(p.body.data["session"]["tainted_by"], json!([session::UNSEEN, "read_journal", "show_ledger"]), "the ledger keeps what it read");
    // The record says what cited it, for what the agent is told.
    let cited: Vec<u64> = session::read(&b, &s.id).unwrap().unwrap().proposals.iter().map(|c| c.proposal).collect();
    assert_eq!(cited, [p.body.seq]);
    // A source that is not a tool's name is refused, and nothing changes.
    assert!(session::taint(&b, &s.id, "x; rm").is_err());
    // Bounded: past so many sources no more are named.
    for i in 0..session::SOURCES_KEPT + 5 {
        session::taint(&b, &s.id, &format!("tool_{i}")).unwrap();
    }
    assert_eq!(session::read(&b, &s.id).unwrap().unwrap().tainted_by.len(), session::SOURCES_KEPT);
}

#[test]
fn a_session_must_be_the_brokers_and_the_agents() {
    let (dir, b) = setup(&[]);
    let d = htop(dir.path());
    let err = format!("{:#}", propose(&b, &d, Actor::Agent, false, Some(&"0".repeat(32))).unwrap_err());
    assert!(err.contains("no session the broker holds"), "{err}");
    assert!(kinds(&b, "refused") == 1, "an agent's refusal is ledgered");
    for bad in ["../../x", &"A".repeat(32), &"0".repeat(31)] {
        let err = format!("{:#}", propose(&b, &d, Actor::Agent, false, Some(bad)).unwrap_err());
        assert!(err.contains("is not a session's id"), "{bad}: {err}");
        assert!(session::read(&b, bad).is_err(), "{bad}");
    }
    let s = session::start(&b).unwrap();
    assert!(propose(&b, &d, Actor::User, false, Some(&s.id)).is_err(), "a person names no session");
    assert!(session::taint(&b, &"1".repeat(32), "read_journal").is_err(), "no such session");
}

/// The review of w5: sessions are bounded in what they keep, and an old one's file going makes a
/// proposal naming it refused. Records written before phase 2 of ADR-055 (they say `resident`) count as any.
#[test]
fn sessions_are_bounded() {
    let (dir, b) = setup(&[]);
    let sessions = dir.path().join("state/sessions");
    fs::create_dir_all(&sessions).unwrap();
    // As many as are kept, from long ago: one more start takes the oldest away.
    for i in 0..session::SESSIONS_KEPT {
        let id = format!("{i:032x}");
        fs::write(sessions.join(format!("{id}.json")), json!({"id": id, "started": format!("2020-01-01T00:{:02}:{:02}Z", i / 60 % 60, i % 60), "tainted": i == 0, "tainted_by": [], "agent": "torq-agent-casey", "resident": i % 2 == 0}).to_string()).unwrap();
    }
    let oldest = format!("{:032x}", 0);
    session::start(&b).unwrap();
    assert_eq!(fs::read_dir(&sessions).unwrap().count(), session::SESSIONS_KEPT);
    assert!(session::read(&b, &oldest).unwrap().is_none(), "the oldest went");
    assert!(propose(&b, &htop(dir.path()), Actor::Agent, false, Some(&oldest)).is_err(), "gone: refused");
    // At most so many started in an hour, whatever kind of record they are.
    let now = chrono::Utc::now().to_rfc3339();
    for i in 0..session::SESSIONS_AN_HOUR - 1 {
        let id = format!("{:032x}", 1_000_000 + i);
        fs::write(sessions.join(format!("{id}.json")), json!({"id": id, "started": now, "tainted": i % 2 == 0, "tainted_by": [], "agent": "torq-agent-casey", "resident": i % 2 == 1}).to_string()).unwrap();
    }
    assert!(format!("{:#}", session::start(&b).unwrap_err()).contains("in the last hour"));
    // Each agent its own count, and its own oldest (ADR-051 3): casey's agent used up, sam's starts, and its
    // start takes none of casey's records away.
    let before = fs::read_dir(&sessions).unwrap().count();
    let sam = as_agent(dir.path(), "torq-agent-sam", &["casey", "sam"]);
    assert_eq!(session::start(&sam).unwrap().agent, "torq-agent-sam");
    assert_eq!(fs::read_dir(&sessions).unwrap().count(), before + 1);
}

/// A session the old loop's resident agent started clean, kept from before phase 2 of ADR-055: its id is in the
/// ledger for every account to read. Cited now, it is tainted as every session is, in its record and in the
/// proposal's entry: nothing of the agent's is clean because a record from before says so.
#[test]
fn a_clean_session_from_before_is_tainted_now() {
    let (dir, b) = setup(&[]);
    let sessions = dir.path().join("state/sessions");
    fs::create_dir_all(&sessions).unwrap();
    let old = format!("{:032x}", 9);
    fs::write(sessions.join(format!("{old}.json")), json!({"id": old, "started": "2026-11-14T10:00:00Z", "tainted": false, "tainted_by": [], "agent": "torq-agent-casey", "proposals": [{"proposal": 3, "applied": 4}], "resident": true, "asks": 0, "denied": []}).to_string()).unwrap();
    let r = session::read(&b, &old).unwrap().expect("still read");
    assert!(r.tainted && r.tainted_by == [session::UNSEEN], "{r:?}");
    let p = propose(&b, &htop(dir.path()), Actor::Agent, false, Some(&old)).unwrap();
    assert!(p.body.tainted && p.body.data["session"]["tainted_by"] == json!([session::UNSEEN]), "{:?}", p.body);
    assert!(session::taint(&b, &old, "read_journal").unwrap().tainted, "and reported into as any");
}

/// The review of ADR-051 step 1: a record from before agents had names is no one's (none may report
/// into it or cite it), so it is not kept: the next session started removes it.
#[test]
fn a_record_of_no_agents_goes() {
    let (dir, b) = setup(&[]);
    let sessions = dir.path().join("state/sessions");
    fs::create_dir_all(&sessions).unwrap();
    let old = format!("{:032x}", 7);
    fs::write(sessions.join(format!("{old}.json")), json!({"id": old, "started": "2026-11-01T00:00:00Z", "tainted": false, "tainted_by": []}).to_string()).unwrap();
    assert_eq!(session::read(&b, &old).unwrap().unwrap().agent, "");
    assert!(session::taint(&b, &old, "read_journal").is_err(), "no one's to report into");
    let s = session::start(&b).unwrap();
    assert!(session::read(&b, &old).unwrap().is_none(), "removed by the next start");
    assert!(session::read(&b, &s.id).unwrap().is_some());
}

/// ADR-051 3: a session is its agent account's: no other agent reports into it or cites it, and an
/// account that is not an agent (none named, or one the map does not name) starts none. sam's agent cannot
/// propose in casey's folder (refused, and ledgered as sam's agent's).
#[test]
fn a_session_is_its_own_agents() {
    let (dir, b) = setup(&[]);
    let b = b.with_agent_limits(&Limits { agents: agents(&["casey", "sam"]), settings: vec![] });
    let sam = as_agent(dir.path(), "torq-agent-sam", &["casey", "sam"]);
    let s = session::start(&b).unwrap();
    assert_eq!(s.agent, "torq-agent-casey");
    assert!(format!("{:#}", session::taint(&sam, &s.id, "read_journal").unwrap_err()).contains("not torq-agent-sam's"));
    assert_eq!(session::read(&b, &s.id).unwrap().unwrap().tainted_by, [session::UNSEEN], "sam's agent reported nothing into it");
    let e = format!("{:#}", propose(&sam, &diff(dir.path(), "s.diff", "users/sam/packages.json", "[\"jq\"]"), Actor::Agent, false, Some(&s.id)).unwrap_err());
    assert!(e.contains("not torq-agent-sam's"), "{e}");
    // sam's agent, in casey's folder: refused, on the record as sam's agent's.
    let own = session::start(&sam).unwrap();
    let e = format!("{:#}", propose(&sam, &htop(dir.path()), Actor::Agent, false, Some(&own.id)).unwrap_err());
    assert!(e.contains("only for sam"), "{e}");
    let last = b.ledger.read_all().unwrap().pop().unwrap();
    assert_eq!((last.body.kind.as_str(), last.body.data["agent"].as_str()), ("refused", Some("torq-agent-sam")));
    // In its own person's folder: a proposal, which names whose agent made it.
    let p = propose(&sam, &diff(dir.path(), "t.diff", "users/sam/packages.json", "[\"jq\"]"), Actor::Agent, false, Some(&own.id)).unwrap();
    assert_eq!((p.body.data["agent"].as_str(), p.body.data["agent_for"].as_str()), (Some("torq-agent-sam"), Some("sam")));
    // No agent named, or one the map does not name: no session, no proposal.
    let none = Broker::new(dir.path().join("config"), dir.path().join("state"), "true").with_agent_limits(&Limits { agents: agents(&["casey"]), settings: vec![] });
    assert!(session::start(&none).is_err());
    assert!(propose(&none, &htop(dir.path()), Actor::Agent, false, None).is_err());
    let stranger = as_agent(dir.path(), "torq-agent-x", &["casey"]);
    assert!(format!("{:#}", session::start(&stranger).unwrap_err()).contains("not an agent"));
    assert!(propose(&stranger, &htop(dir.path()), Actor::Agent, false, None).is_err());
    // The old loop's second account (`torq-resident-<person>`, removed) is no agent either.
    let resident = as_agent(dir.path(), "torq-resident-casey", &["casey"]);
    assert!(format!("{:#}", session::start(&resident).unwrap_err()).contains("not an agent"));
    assert!(propose(&resident, &htop(dir.path()), Actor::Agent, false, None).is_err());
}

/// ADR-042, 6: an agent's change that could lose the network is applied only as a commit confirmed.
#[test]
fn what_could_cut_the_machine_off_is_applied_confirmed() {
    let (dir, b) = setup(&["networking"]);
    let s = session::start(&b).unwrap();
    let wifi = diff(dir.path(), "w.diff", "nixos/networking.json", "{\"wireless\": {\"enable\": true}}");
    let p = propose(&b, &wifi, Actor::Agent, false, Some(&s.id)).unwrap();
    assert_eq!(p.body.data["cuts_off"], json!(["networking.wireless.enable"]));
    let err = format!("{:#}", b.apply(p.body.seq, "casey", true).unwrap_err());
    assert!(err.contains("commit confirmed"), "{err}");
    assert_eq!(kinds(&b, "applied"), 0);
    // A person's own change is theirs to make as they like.
    let mine = propose(&b, &diff(dir.path(), "m.diff", "nixos/time.json", "{\"timeZone\": \"UTC\"}"), Actor::User, false, None).unwrap();
    assert!(mine.body.data.get("cuts_off").is_none());
}

/// A commit at never takes an agent's proposal: it is read and applied by a person who is there. Nor (the
/// review of w5) one that could cut the machine off, which an agent's proposal left clean in a ledger from
/// before phase 2 of ADR-055 still could be: no one is there to confirm.
#[test]
fn a_commit_at_refuses_an_agents_proposal_and_what_could_cut_the_machine_off() {
    let (dir, b) = setup(&["networking"]);
    let s = session::start(&b).unwrap();
    let wifi = diff(dir.path(), "w.diff", "nixos/networking.json", "{\"wireless\": {\"enable\": true}}");
    let p = propose(&b, &wifi, Actor::Agent, false, Some(&s.id)).unwrap();
    let at = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    let sched = torq_broker::at::Scheduler::new(&b, "true", "true");
    let err = format!("{:#}", sched.schedule(p.body.seq, "casey", &at).unwrap_err());
    assert!(err.contains("is an agent's"), "{err}");
    // As an older broker wrote one from the resident agent: not tainted, and it could cut the machine off.
    let mut data = p.body.data.clone();
    data["resident"] = true.into();
    let old = b.ledger.append(NewEntry { actor: "agent".into(), approved_by: None, kind: "proposal".into(), tier: p.body.tier.clone(), tainted: false, reason: "wifi".into(), data }).unwrap();
    let err = format!("{:#}", sched.schedule(old.body.seq, "casey", &at).unwrap_err());
    assert!(err.contains("could lose the network or the screen"), "{err}");
    assert_eq!(kinds(&b, "apply_scheduled"), 0);
}

/// Phase 2 of ADR-055: session trust is gone. A grant of it that an older broker ledgered, still inside its
/// minutes, is on the record and lets nothing through: it is no grant that holds, an agent's proposal made
/// under it is only a proposal, and no entry of this broker's settles or answers it. A person applies.
#[test]
fn a_grant_of_trust_from_before_applies_nothing() {
    let (dir, b) = setup(&[]);
    let until = (chrono::Utc::now() + chrono::Duration::minutes(50)).to_rfc3339();
    for data in [json!({ "person": "casey", "minutes": 60, "until": until }), json!({ "person": "casey", "minutes": 60, "until": until, "covers": "proposals" })] {
        b.ledger.append(NewEntry { actor: "user".into(), approved_by: Some("casey".into()), kind: "trust_granted".into(), tier: "user".into(), tainted: false, reason: "casey lets the agent's proposals in their own folder apply without asking, for 60 minutes".into(), data }).unwrap();
    }
    assert!(session::active(&b.ledger.read_all().unwrap()).is_empty(), "no grant that holds");
    let s = session::start(&b).unwrap();
    let p = propose(&b, &htop(dir.path()), Actor::Agent, false, Some(&s.id)).unwrap();
    assert!(p.body.tainted);
    let after = b.ledger.read_all().unwrap();
    assert_eq!(after.last().unwrap().body.seq, p.body.seq, "the proposal is the last thing on the record");
    assert_eq!((kinds(&b, "applied"), kinds(&b, "trust_expired")), (0, 0));
    assert!(session::read(&b, &s.id).unwrap().unwrap().proposals == [session::Cited { proposal: p.body.seq }]);
    b.ledger.verify().unwrap();
}

/// ADR-051 and ADR-052 4, amended 2026-11-13: a grant covers secret uses, and nothing else. Under one, however
/// wide, no proposal applies: a change stays a proposal a person applies.
#[test]
fn a_grant_of_secret_uses_applies_no_proposal() {
    let (dir, b) = setup(&[]);
    let secrets = session::grant_secret(&b, "casey", session::Covers::AllSecrets, session::Scope::Minutes(480), false, false).unwrap();
    let s = session::start(&b).unwrap();
    let p = propose(&b, &htop(dir.path()), Actor::Agent, false, Some(&s.id)).unwrap();
    assert_eq!(b.ledger.read_all().unwrap().last().unwrap().body.seq, p.body.seq, "it waits for a person");
    assert_eq!(kinds(&b, "applied"), 0);
    assert_eq!(session::active(&b.ledger.read_all().unwrap()).iter().map(|g| g.seq).collect::<Vec<_>>(), [secrets.body.seq]);
    // No agent of theirs: nothing to let it use.
    assert!(format!("{:#}", session::grant_secret(&b, "pat", session::Covers::AllSecrets, session::Scope::Minutes(10), false, false).unwrap_err()).contains("has no agent"));
}

/// A ledger an older broker wrote, with session trust in it (granted, revoked, expired, and a grant from
/// before grants said what they cover): every entry verifies as it was written, and none of its grants is
/// one this broker honours.
#[test]
fn a_ledger_with_session_trust_verifies_and_grants_nothing() {
    let text = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../wire/tests/fixtures/session-trust.jsonl")).unwrap();
    let entries: Vec<torq_broker::ledger::Entry> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    torq_broker::ledger::verify_entries(&entries).unwrap();
    let at = entries.iter().position(|e| e.body.kind == "trust_granted").expect("the fixture has a grant");
    assert!(entries[at].body.data.get("covers").is_none(), "written before grants said what they cover");
    let then = &entries[..=at];
    torq_broker::ledger::verify_entries(then).unwrap();
    assert!(session::unsettled(then).is_empty(), "trust in proposals: no grant now");
    assert!(session::unsettled(&entries).is_empty() && session::active(&entries).is_empty());
}

/// A ledger the broker wrote before phase 2 of ADR-055 (`wire/tests/fixtures/before-phase-2.jsonl`, made by
/// that broker's own code): session trust granted, a proposal of the old loop's resident agent applied under
/// it (`data.trust`), proposals whose sessions say `not-the-resident-agent`, `coding-tool` and
/// `local-tcp-model`, a refusal marked `resident`, callers named `torq-resident-<person>`, the trust revoked,
/// and one more grant left standing. This broker writes none of those words. It still reads every line as it
/// was written and verifies the chain, so `torq-broker verify` passes on a machine that has them; and the
/// grant left standing is no grant: nothing is applied under it.
#[test]
fn a_ledger_from_before_phase_2_verifies_and_its_trust_grants_nothing() {
    let text = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../wire/tests/fixtures/before-phase-2.jsonl")).unwrap();
    for word in ["\"trust\":2", "\"resident\":true", "not-the-resident-agent", "coding-tool", "local-tcp-model", "torq-resident-casey", "\"covers\":\"proposals\"", "\"kind\":\"trust_revoked\""] {
        assert!(text.contains(word), "the fixture has no {word}");
    }
    let entries: Vec<torq_broker::ledger::Entry> = text
        .lines()
        .map(|l| {
            let e: torq_broker::ledger::Entry = serde_json::from_str(l).unwrap();
            assert_eq!(serde_json::to_string(&e).unwrap(), l, "read and written back as it was");
            e
        })
        .collect();
    torq_broker::ledger::verify_entries(&entries).unwrap();
    // The same, as a machine's own ledger file: the broker's `verify`, and an entry appended after the old ones.
    let (dir, b) = setup(&[]);
    fs::write(b.ledger.path(), &text).unwrap();
    b.ledger.verify().unwrap();
    let last = entries.last().unwrap();
    assert_eq!((last.body.kind.as_str(), last.body.data["covers"].as_str()), ("trust_granted", Some("proposals")), "a grant left standing");
    assert!(session::unsettled(&entries).is_empty(), "it is no grant now, whatever time it is");
    let s = session::start(&b).unwrap();
    let p = b.propose_in(None, &htop(dir.path()), "install htop", Actor::Agent, false, Some(&s.id));
    // (Refused or taken, as this scratch machine's repository has it: either way nothing is applied.)
    let after = b.ledger.read_all().unwrap();
    assert!(after.iter().skip(entries.len()).all(|e| e.body.kind != "applied" && e.body.kind != "trust_expired"), "{p:?}");
    b.ledger.verify().unwrap();
}
