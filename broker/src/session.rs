//! The agent's sessions, and a person's leave for their agent to use a secret (ADR-042 6 amended, ADR-052 4).
//!
//! **Sessions.** A program run as an agent account (`torq-agent-<person>`, ADR-051 3: the person's coding
//! tool, through `torq-mcp`) starts a session at the broker over its socket; the broker makes its record
//! here, in its own state directory (root's, 0600), naming that account: a session is its account's alone
//! — no other agent reports into it or cites it. A proposal of the agent's names its session, and the
//! proposal's entry in the ledger carries the session's id, when it started and what it reported reading —
//! so a session that proposed nothing costs the ledger nothing, and the record of one that did outlives its
//! file here.
//!
//! The broker cannot see what a program of an agent's reads (the web, a file, what its model was told). So
//! every session counts from its start as one that read untrusted text (its record begins with `UNSEEN`; in
//! the ledger an entry of an agent's is `tainted`, the ledger's own word, kept), and every proposal of an agent's is too, whatever
//! session it names or none: it waits for a person, who reads it and approves it, one by one. Nothing here
//! applies an agent's proposal without a person; there is no trust to give that would (session trust,
//! ADR-042 7, was removed with TorqOS's own agent loop: ADR-055, phase 2). What a program reports of its
//! reads (`session-taint`, a tool's name) is added to the record, for the person who reads the proposal.
//!
//! **Leave to use a secret.** A person lets their own agent use one secret without asking, for some minutes:
//! `trust_granted` with what it covers, the ledger the only record of it.
//! Grants, revokes and expiries are written under the sessions' own short lock, never the broker's: a
//! revoke is on the record at once. Its time ends it (`trust_expired`, ledgered by the next operation that
//! reads the grants — it is over at its time whether or not that has been written).
//!
//! **Older records.** A ledger from before phase 2 holds entries this broker no longer writes: a
//! `trust_granted` that covers proposals (or says nothing of what it covers), its `trust_revoked` and
//! `trust_expired`, an `applied` entry with `data.trust`, a proposal whose `data.session.tainted_by` names
//! `not-the-resident-agent` or a model's word, `data.resident`. `verify` checks the chain of hashes and
//! reads none of that; here such a grant covers nothing (`Covers::read`), and neither do the kinds of secret
//! grant that went on 2026-11-16 (all secrets, "this session", one that did not hold under untrusted text).
//! A session record from before is
//! read with the fields it had then left out (`parse`).

use crate::ledger::{Entry, NewEntry};
use crate::policy::{self, Actor, Tier};
use crate::{Broker, Lock};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::{Read, Write};
use std::path::PathBuf;

/// How many sessions may be started in an hour: past that, refused. Each agent its own count.
pub const SESSIONS_AN_HOUR: usize = 600;
/// How many session records are kept for each agent: past that, its oldest go. A proposal naming a session whose
/// record is gone is refused; what it had read is in its proposal's entry.
pub const SESSIONS_KEPT: usize = 2000;

/// What the broker holds of a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub started: String,
    /// What it read: `UNSEEN` first (the broker cannot see what a program of an agent's reads, so every session
    /// counts as one that read untrusted text, from its start), then each tool's name it reported.
    pub tainted_by: Vec<String>,
    /// The agent account whose session it is (ADR-051 3). Empty (a record from before): no agent's,
    /// removed by the next session started,
    /// so no agent reports into it or cites it.
    #[serde(default)]
    pub agent: String,
    /// The proposals that cited it, as the broker ledgered them (what the agent is told it did).
    #[serde(default)]
    pub proposals: Vec<Cited>,
    /// How many of its secret uses a person has been asked about (ask.rs): past the most, no more are asked.
    #[serde(default)]
    pub asks: u32,
    /// The secrets a person said not to ask about again in this session, each as `Owner::key` says it.
    #[serde(default)]
    pub denied: Vec<String>,
}

/// A proposal that cited a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cited {
    pub proposal: u64,
}

/// What every session of an agent account's is marked with from its start, and every proposal of one: the
/// broker cannot see what a program run as an agent reads.
pub const UNSEEN: &str = "agent-program";

/// A session's id: 128 random bits, as 32 lowercase hex digits.
pub fn id_ok(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// What a reported read may be called: a tool's name.
fn source_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn dir(b: &Broker) -> PathBuf {
    b.state_dir.join("sessions")
}

/// The record of session `id`, if there is one (never through a link).
pub fn read(b: &Broker, id: &str) -> Result<Option<Record>> {
    use std::os::unix::fs::OpenOptionsExt;
    if !id_ok(id) {
        bail!("{id:?} is not a session's id");
    }
    let p = dir(b).join(format!("{id}.json"));
    let f = match std::fs::OpenOptions::new().read(true).custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32).open(&p) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", p.display())),
    };
    let mut s = String::new();
    f.take(64 * 1024).read_to_string(&mut s)?;
    Ok(Some(parse(&s).with_context(|| format!("reading {}", p.display()))?))
}

/// A record's text, read. One written before phase 2 of ADR-055 says two things more, which are left out
/// before it is read as strictly as ever: whether the resident agent started it (`resident`), and for each
/// proposal the change a person's trust made of it (`applied`). Neither exists any more. And every record
/// until 2026-11-16 said whether its session had read untrusted text (`tainted`): every session counts as
/// one that did, so the record no longer says it, and one that said it had not (the resident agent's, started
/// clean) is read as every other, with `UNSEEN` first in what it read: it is not left for another program
/// to cite as clean (its id is in the ledger for every account to read).
fn parse(text: &str) -> Result<Record> {
    let mut v: serde_json::Value = serde_json::from_str(text)?;
    let mut was_clean = false;
    if let Some(o) = v.as_object_mut() {
        o.remove("resident");
        was_clean = o.remove("tainted") == Some(serde_json::Value::Bool(false));
        for c in o.get_mut("proposals").and_then(|p| p.as_array_mut()).into_iter().flatten() {
            if let Some(c) = c.as_object_mut() {
                c.remove("applied");
            }
        }
    }
    let mut r: Record = serde_json::from_value(v)?;
    if was_clean {
        r.tainted_by.insert(0, UNSEEN.into());
    }
    Ok(r)
}

/// Write a record: a new file beside it, then renamed over it — a record is whole or as it was.
fn write(b: &Broker, r: &Record) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let d = dir(b);
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&d)?;
    let part = d.join(format!(".{}.new", r.id));
    let _ = std::fs::remove_file(&part);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&part)?;
    f.write_all(&serde_json::to_vec(r)?)?;
    f.sync_all()?;
    std::fs::rename(&part, d.join(format!("{}.json", r.id)))?;
    std::fs::File::open(&d)?.sync_all()?;
    Ok(())
}

/// The sessions' and grants' own lock: short, never the broker's (which a rebuild holds for
/// minutes), so that a report or a revoke is never kept waiting by a change. Never taken twice in
/// one process at once; may be taken while the broker's is held, never the other way round.
fn lock(b: &Broker) -> Result<Lock> {
    Lock::acquire_named(&b.state_dir, "sessions.lock", 10)
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Every record kept, as (when it started, its id, whose).
fn kept(b: &Broker) -> Result<Vec<(String, String, String)>> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir(b)) else { return Ok(out) };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".json").filter(|i| id_ok(i)) else { continue };
        if let Some(r) = read(b, id)? {
            out.push((r.started, r.id, r.agent));
        }
    }
    out.sort();
    Ok(out)
}

/// Start a session of the acting agent's (`Broker::as_agent`): its record, here, naming that account.
/// At most `SESSIONS_AN_HOUR` an hour for each agent; past
/// `SESSIONS_KEPT`, the oldest records go.
pub fn start(b: &Broker) -> Result<Record> {
    let (agent, _) = b.acting_agent()?;
    let _l = lock(b)?;
    let kept = kept(b)?;
    let hour_ago = chrono::Utc::now() - chrono::Duration::hours(1);
    // Each agent its own count: one person's agent cannot keep another's from starting.
    let recent = kept.iter().filter(|(t, _, a)| a == agent && chrono::DateTime::parse_from_rfc3339(t).is_ok_and(|t| t > hour_ago)).count();
    if recent >= SESSIONS_AN_HOUR {
        bail!("refused: {recent} of {agent}'s sessions were started in the last hour (at most {SESSIONS_AN_HOUR})");
    }
    // A record of no agent's (from before ADR-051 3) is no one's to report into or cite: it goes. Then its
    // own oldest go, never another agent's.
    for (_, id, _) in kept.iter().filter(|(_, _, a)| a.is_empty()) {
        std::fs::remove_file(dir(b).join(format!("{id}.json")))?;
    }
    let mine: Vec<&(String, String, String)> = kept.iter().filter(|(_, _, a)| a == agent).collect();
    for (_, id, _) in mine.iter().take((mine.len() + 1).saturating_sub(SESSIONS_KEPT)) {
        std::fs::remove_file(dir(b).join(format!("{id}.json")))?;
    }
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).context("reading /dev/urandom")?;
    let id: String = bytes.iter().map(|x| format!("{x:02x}")).collect();
    if read(b, &id)?.is_some() {
        bail!("a session {id} exists already");
    }
    let r = Record { id, started: now(), tainted_by: vec![UNSEEN.into()], agent: agent.into(), proposals: vec![], asks: 0, denied: vec![] };
    write(b, &r)?;
    Ok(r)
}

/// How many distinct sources a session's record names; past them no more are added.
pub const SOURCES_KEPT: usize = 32;

/// Session `id` read untrusted text (from `source`, a tool's name): added to its record, for good. Only by
/// the agent whose it is.
pub fn taint(b: &Broker, id: &str, source: &str) -> Result<Record> {
    if !source_ok(source) {
        bail!("{source:?} is not a source's name (a tool's: letters, digits, - and _)");
    }
    let _l = lock(b)?;
    let (agent, _) = b.acting_agent()?;
    let mut r = read(b, id)?.context("refused: no such session")?;
    if r.agent != agent {
        bail!("refused: session {id} is not {agent}'s");
    }
    // Bounded: a record any agent program can report into must not grow until no one can start a
    // session (the audit of 0.3.0). Past the bound, what else it read is not named.
    if !r.tainted_by.iter().any(|s| s == source) && r.tainted_by.len() < SOURCES_KEPT {
        r.tainted_by.push(source.into());
    }
    write(b, &r)?;
    Ok(r)
}

/// Whether a proposal is tainted, and what of its session goes into its entry. A person's proposal names no
/// session, and is tainted only if the one who made it says so (`flag`). An agent's is tainted, always
/// (the module's words): with a session, its entry carries what the session reported reading; one that
/// names a session not held here, or another agent's, is refused.
pub fn judge(b: &Broker, actor: Actor, flag: bool, session: Option<&str>) -> Result<(bool, Option<serde_json::Value>)> {
    match (actor, session) {
        (Actor::User, Some(_)) => bail!("refused: a session is the agent's; a person's proposal names none"),
        (Actor::User, None) => Ok((flag, None)),
        (Actor::Agent, None) => {
            b.acting_agent()?;
            Ok((true, None))
        }
        (Actor::Agent, Some(id)) => {
            let (agent, _) = b.acting_agent()?;
            let r = read(b, id)?.context("refused: the proposal names no session the broker holds")?;
            // Its own session, or none: one agent never cites another's (ADR-051 3).
            if r.agent != agent {
                bail!("refused: session {id} is not {agent}'s");
            }
            Ok((true, Some(json!({ "id": r.id, "started": r.started, "tainted_by": r.tainted_by }))))
        }
    }
}

/// The proposal `seq` cited session `id`: noted in its record, for what the agent is told.
pub fn cite(b: &Broker, id: &str, seq: u64) -> Result<()> {
    let _l = lock(b)?;
    let Some(mut r) = read(b, id)? else { return Ok(()) };
    if !r.proposals.iter().any(|c| c.proposal == seq) {
        r.proposals.push(Cited { proposal: seq });
        write(b, &r)?;
    }
    Ok(())
}

/// A person is asked about one more secret use of session `id`: counted in its record, and the count given.
pub(crate) fn count_ask(b: &Broker, id: &str) -> Result<u32> {
    let _l = lock(b)?;
    let mut r = read(b, id)?.context("refused: no such session")?;
    r.asks = r.asks.saturating_add(1);
    write(b, &r)?;
    Ok(r.asks)
}

/// A person said not to ask again in session `id` about the secret `key`: its uses are refused at once from now.
/// A session whose record is gone has nothing left to refuse.
pub(crate) fn deny_rest(b: &Broker, id: &str, key: &str) -> Result<()> {
    let _l = lock(b)?;
    let Some(mut r) = read(b, id)? else { return Ok(()) };
    if !r.denied.iter().any(|k| k == key) {
        r.denied.push(key.into());
        write(b, &r)?;
    }
    Ok(())
}

/// The longest a person's grant of a secret's uses lasts.
pub const SECRET_GRANT_MAX_MINUTES: u32 = 480;

/// What a grant covers: the agent's uses of one secret, the person's own of that name or (`system`) the
/// system's. A grant is of one named secret's uses: none applies a proposal, and none is of every secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Covers {
    pub name: String,
    pub system: bool,
}

impl Covers {
    /// As a `trust_*` entry says it (`data.covers`).
    fn word(&self) -> String {
        format!("secret {}", self.name)
    }

    /// From a `trust_granted` entry's data: a grant this broker honours, or None. The one kind there is: one
    /// named secret, for minutes, whatever the agent has read (the entry's `tainted_too`, the ledger's word
    /// for it since before every use of an agent's counted as following untrusted text; still written, so
    /// that an entry says the same thing to every broker that reads it).
    ///
    /// None, and so no grant, for everything else a ledger may hold: what this broker does not know; trust in
    /// proposals from before phase 2 of ADR-055 (`proposals`, or nothing said); and the kinds of secret grant
    /// that went on 2026-11-16 (ADR-052 4, amended), each of which let no agent's use through by then: all of
    /// a person's secrets (`all-secrets`), one given for "this session" (`scope: session`), and one that did
    /// not say `tainted_too`. Such an entry stays on the record, verifies, and lets nothing through.
    fn read(data: &serde_json::Value) -> Option<Covers> {
        let name = data.get("covers")?.as_str()?.strip_prefix("secret ").filter(|n| torq_wire::secrets::name_ok(n))?;
        (data["tainted_too"] == json!(true) && data.get("scope").is_none()).then(|| Covers { name: name.into(), system: data["owner"] == json!("system") })
    }
}

/// A person's grant to their own agent, not yet revoked or expired on the record: leave to use one secret
/// without asking, for some minutes (ADR-051, amended 2026-11-13; ADR-052 4, amended 2026-11-16).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Grant {
    /// Its `trust_granted` entry.
    pub seq: u64,
    pub person: String,
    pub minutes: u64,
    pub until: String,
    /// When it was granted: its entry's time. A grant lasts no longer than its minutes from then,
    /// whatever its `until` says (a ledger from elsewhere is someone else's text: the audit of 0.3.0).
    pub at: String,
    pub covers: Covers,
}

impl Grant {
    fn over(&self, at: chrono::DateTime<chrono::Utc>) -> bool {
        let minutes = i64::try_from(self.minutes.min(u64::from(SECRET_GRANT_MAX_MINUTES))).unwrap_or(0);
        let ends = chrono::DateTime::parse_from_rfc3339(&self.at).map(|t| t + chrono::Duration::minutes(minutes));
        let until = chrono::DateTime::parse_from_rfc3339(&self.until);
        match (until, ends) {
            (Ok(u), Ok(e)) => u.min(e) <= at,
            _ => true,
        }
    }

    /// Whether it lets the agent use the secret `name` (the system's, or its person's own) without asking, at `at`.
    pub fn covers_use(&self, system: bool, name: &str, at: chrono::DateTime<chrono::Utc>) -> bool {
        self.covers.name == name && self.covers.system == system && !self.over(at)
    }
}

/// Every grant not settled on the record (revoked or expired), oldest first.
pub fn unsettled(entries: &[Entry]) -> Vec<Grant> {
    let settled = |seq: u64| entries.iter().any(|e| matches!(e.body.kind.as_str(), "trust_revoked" | "trust_expired") && e.body.data["grant"] == json!(seq));
    entries
        .iter()
        .filter(|e| e.body.kind == "trust_granted" && !settled(e.body.seq))
        .filter_map(|e| {
            let covers = Covers::read(&e.body.data)?;
            Some(Grant {
                seq: e.body.seq,
                person: e.body.data["person"].as_str()?.into(),
                minutes: e.body.data["minutes"].as_u64()?,
                until: e.body.data["until"].as_str()?.into(),
                at: e.body.ts.clone(),
                covers,
            })
        })
        .collect()
}

/// The grants that hold now: unsettled, and their time not over.
pub fn active(entries: &[Entry]) -> Vec<Grant> {
    let now = chrono::Utc::now();
    unsettled(entries).into_iter().filter(|g| !g.over(now)).collect()
}

/// Ledger `trust_expired` for every grant whose time is over and is not yet settled, under the
/// sessions' lock (held).
fn settle_locked(b: &Broker) -> Result<()> {
    let now = chrono::Utc::now();
    for g in unsettled(&b.ledger.read_all()?).into_iter().filter(|g| g.over(now)) {
        let what = format!("leave for the agent to use {} without asking", said(&g.covers));
        b.ledger.append(NewEntry {
            actor: "broker".into(),
            approved_by: None,
            kind: "trust_expired".into(),
            tier: Tier::User.to_string(),
            tainted: false,
            reason: format!("{}'s {what} ended at {}", g.person, g.until),
            data: json!({ "grant": g.seq, "person": g.person, "until": g.until, "covers": g.covers.word() }),
        })?;
    }
    Ok(())
}

/// What a grant covers, as a person reads it.
fn said(c: &Covers) -> String {
    if c.system {
        format!("the system's secret {}", c.name)
    } else {
        format!("their secret {}", c.name)
    }
}

/// The grant among `active` that `person` has of exactly `covers`, if any: the one a second would meet.
fn held<'a>(active: &'a [Grant], person: &str, covers: &Covers) -> Option<&'a Grant> {
    active.iter().find(|g| g.person == person && g.covers == *covers)
}

/// `person` lets their own agent use one secret (`covers`) without asking, for `minutes` (1 to
/// `SECRET_GRANT_MAX_MINUTES`), whatever the agent reads meanwhile. One of each secret at a time: where one
/// holds already it is refused, or (`replace`) ended in the same step that makes the new one. (Over the
/// socket: only that person, with their password.) What it never does: it gives the agent no secret its rule
/// does not — a use is still judged by its rule first, its host and its actors (`secrets::may_use`). Under the
/// sessions' lock, never the broker's.
pub fn grant_secret(b: &Broker, person: &str, covers: Covers, minutes: u32, replace: bool) -> Result<Entry> {
    use torq_wire::secrets::AGENT;
    if !(1..=SECRET_GRANT_MAX_MINUTES).contains(&minutes) {
        bail!("refused: such a grant lasts 1 to {SECRET_GRANT_MAX_MINUTES} minutes, not {minutes}");
    }
    if !policy::user_name_ok(person) {
        bail!("{person:?} is not a person's name");
    }
    if !b.agent_limits.has_agent(person) {
        bail!("refused: {person} has no agent (torq.agent.people), so there is nothing to trust it with");
    }
    // A grant widens no rule: said here so that a person is not left thinking it did.
    let owner = if covers.system { crate::secrets::Owner::System } else { crate::secrets::Owner::Person(person.into()) };
    let rules = b.rules_of(&owner)?;
    let rule = rules.get(&covers.name).with_context(|| match covers.name.as_str() {
        // What `allow all for <minutes>` once meant, said to someone who learned it: there is no secret of that name.
        "all" => "refused: allowing every secret at once is gone. Name the one secret: request system secret allow <name> for <minutes> (show system secrets lists them)".to_string(),
        name => format!("refused: no rule for a secret {name} of {}", if covers.system { "the system's" } else { "theirs" }),
    })?;
    let named = if covers.system { b.agent_limits.agents.iter().any(|(a, p)| p == person && rule.actors.contains(a)) } else { rule.actors.iter().any(|a| a == AGENT) };
    if !named {
        bail!("refused: the rule of secret {} does not let {person}'s agent use it: a grant widens no rule", covers.name);
    }
    let _l = lock(b)?;
    settle_locked(b)?;
    // Where a grant of this holds already: refused, or (`replace`) ended in the same step that makes the new one,
    // so that a refusal before this point leaves the old one as it was.
    if let Some(g) = held(&active(&b.ledger.read_all()?), person, &covers) {
        if !replace {
            bail!("refused: {person} lets the agent use {} already, until {} (#{}): revoke it first", said(&covers), g.until, g.seq);
        }
        revoke_locked(b, g)?;
    }
    let until = (chrono::Utc::now() + chrono::Duration::minutes(minutes.into())).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    // (`tainted_too`: the ledger's word for the one kind of grant there is, see `Covers::read`.)
    let data = json!({ "person": person, "minutes": minutes, "until": until, "covers": covers.word(), "owner": if covers.system { "system" } else { "user" }, "tainted_too": true });
    let reason = format!("{person} lets the agent use {} without asking, for {minutes} minutes", said(&covers));
    b.ledger.append(NewEntry { actor: Actor::User.to_string(), approved_by: Some(person.into()), kind: "trust_granted".into(), tier: Tier::User.to_string(), tainted: false, reason, data })
}

/// The grant that lets `person`'s agent make this use without asking, if one holds (`Grant::covers_use`): its entry.
pub fn covering(b: &Broker, person: &str, system: bool, name: &str) -> Result<Option<u64>> {
    let _l = lock(b)?;
    settle_locked(b)?;
    let now = chrono::Utc::now();
    Ok(active(&b.ledger.read_all()?).into_iter().find(|g| g.person == person && g.covers_use(system, name, now)).map(|g| g.seq))
}

/// `trust_revoked` for grant `g`, by its person. Under the sessions' lock (held).
fn revoke_locked(b: &Broker, g: &Grant) -> Result<Entry> {
    let what = format!("leave for the agent to use {} without asking", said(&g.covers));
    b.ledger.append(NewEntry {
        actor: Actor::User.to_string(),
        approved_by: Some(g.person.clone()),
        kind: "trust_revoked".into(),
        tier: Tier::User.to_string(),
        tainted: false,
        reason: format!("{} revoked their {what}", g.person),
        data: json!({ "grant": g.seq, "person": g.person, "covers": g.covers.word() }),
    })
}

/// Which of a person's grants to end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    /// Every one they gave.
    Everything,
    /// The grant of one secret: their own of that name, or (`system`) the system's. Never both: a person's own
    /// secret and a system secret may share a name, and are told apart here as a grant tells them apart.
    Secret { name: String, system: bool },
}

/// End `person`'s grants now, those `which` names. No use starts under them after.
pub fn revoke_secrets(b: &Broker, person: &str, which: &Ending) -> Result<Vec<Entry>> {
    let _l = lock(b)?;
    settle_locked(b)?;
    let theirs = |g: &Grant| match which {
        Ending::Everything => true,
        Ending::Secret { name, system } => g.covers.name == *name && g.covers.system == *system,
    };
    active(&b.ledger.read_all()?).iter().filter(|g| g.person == person && theirs(g)).map(|g| revoke_locked(b, g)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gh(system: bool) -> Covers {
        Covers { name: "gh".into(), system }
    }

    fn grant(covers: Covers) -> Grant {
        Grant { seq: 1, person: "casey".into(), minutes: 480, until: "2026-11-08T20:00:00Z".into(), at: "2026-11-08T12:00:00Z".into(), covers }
    }

    fn at(t: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(t).unwrap().with_timezone(&chrono::Utc)
    }

    /// A grant lasts its minutes from when it was made, whatever its `until` says: a ledger from
    /// elsewhere is someone else's text (the audit of 0.3.0).
    #[test]
    fn a_grant_lasts_its_minutes_whatever_its_until_says() {
        let g = |at: &str, minutes: u64, until: &str| Grant { minutes, until: until.into(), at: at.into(), ..grant(gh(false)) };
        let now = at("2026-11-08T12:00:00Z");
        assert!(g("2026-11-08T10:00:00Z", 60, "2099-01-01T00:00:00Z").over(now), "60 minutes from 10:00 is over at 12:00");
        assert!(!g("2026-11-08T11:30:00Z", 60, "2099-01-01T00:00:00Z").over(now));
        assert!(g("2026-11-08T11:30:00Z", 6000, "2099-01-01T00:00:00Z").over(now + chrono::Duration::minutes(i64::from(SECRET_GRANT_MAX_MINUTES) + 1)), "never more than the most a grant may be");
        assert!(g("2026-11-08T11:30:00Z", 60, "2026-11-08T11:45:00Z").over(now), "nor past its until");
        assert!(g("not a time", 60, "2099-01-01T00:00:00Z").over(now), "unreadable is over");
    }

    /// A grant covers the one secret it names: one secret is not another, and a person's own of a name is not
    /// the system's of that name.
    #[test]
    fn a_grant_covers_the_secret_it_names_and_nothing_else() {
        let now = at("2026-11-08T12:30:00Z");
        let own = grant(gh(false));
        assert!(own.covers_use(false, "gh", now));
        assert!(!own.covers_use(false, "other", now), "another secret");
        assert!(!own.covers_use(true, "gh", now), "the system's of the same name");
        let system = grant(gh(true));
        assert!(system.covers_use(true, "gh", now) && !system.covers_use(false, "gh", now));
        assert!(!own.covers_use(false, "gh", at("2026-11-08T20:00:00Z")), "over at its time");
    }

    /// A second grant meets the one that holds of exactly the same secret and the same person, and no other: the
    /// one `--replace` ends in the same step, and that a plain grant is refused for.
    #[test]
    fn a_grant_meets_only_the_same_persons_grant_of_the_same_secret() {
        let mine = grant(gh(false));
        let others = [mine.clone(), Grant { person: "sam".into(), ..mine.clone() }, grant(gh(true)), grant(Covers { name: "ci".into(), system: false })];
        assert_eq!(held(&others, "casey", &gh(false)).map(|g| g.person.as_str()), Some("casey"));
        assert!(std::ptr::eq(held(&others, "casey", &gh(false)).unwrap(), &others[0]), "their own, not sam's");
        assert!(held(&others[1..], "casey", &gh(false)).is_none(), "sam's, the system's and another secret are not it");
    }

    /// What an entry grants. The one kind: a named secret, for minutes, whatever the agent has read (the entry
    /// says `tainted_too`, as this broker writes it). Everything else is no grant: what no torq knows; trust in
    /// proposals from before phase 2 of ADR-055; and the kinds of secret grant that went on 2026-11-16, each
    /// of which let no agent's use through by then (all secrets at once, "this session", and one that did not
    /// say `tainted_too`).
    #[test]
    fn what_an_entry_grants() {
        let one = |owner: &str| json!({ "covers": "secret gh", "owner": owner, "tainted_too": true, "minutes": 60 });
        assert_eq!(Covers::read(&one("user")), Some(gh(false)));
        assert_eq!(Covers::read(&one("system")), Some(gh(true)));
        assert_eq!(Covers::read(&json!({ "person": "casey", "minutes": 30 })), None, "an old grant of trust in proposals");
        assert_eq!(Covers::read(&json!({ "covers": "proposals" })), None, "and one that says so");
        assert_eq!(Covers::read(&json!({ "covers": "all-secrets" })), None, "all of a person's secrets: gone");
        assert_eq!(Covers::read(&json!({ "covers": "all-secrets", "tainted_too": true })), None, "whatever else it says");
        assert_eq!(Covers::read(&json!({ "covers": "secret gh", "owner": "user", "minutes": 60 })), None, "one that did not hold under untrusted text: gone");
        assert_eq!(Covers::read(&json!({ "covers": "secret gh", "owner": "user", "tainted_too": false })), None);
        assert_eq!(Covers::read(&json!({ "covers": "secret gh", "owner": "user", "scope": "session" })), None, "one for \"this session\": gone");
        assert_eq!(Covers::read(&json!({ "covers": "secret gh", "owner": "user", "scope": "session", "tainted_too": true })), None, "which no broker wrote; still none");
        for unknown in [json!({ "covers": "everything", "tainted_too": true }), json!({ "covers": "secret ../x", "tainted_too": true }), json!({ "covers": 7, "tainted_too": true }), json!({ "covers": "secret ", "tainted_too": true })] {
            assert_eq!(Covers::read(&unknown), None, "{unknown}");
        }
    }

    /// A ledger the broker wrote before 2026-11-16 (`wire/tests/fixtures/secret-grants-before.jsonl`, made by
    /// that broker's own code), with every kind of secret grant it gave: all of alice's secrets, one named for
    /// minutes, one named for "this session", one named for minutes "even from a session that read untrusted
    /// text" (hers, and a system secret's), and all of bob's, revoked. Each line is read and written back as it
    /// was and the chain verifies. Of the grants left standing only the last kind is one, as only it let a use
    /// of an agent's through then: nothing became looser, and the others are no grant to anything that reads
    /// them (a use, a second grant, a revoke, an adoption).
    #[test]
    fn a_ledger_of_older_secret_grants_verifies_and_only_the_one_kind_is_a_grant() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../wire/tests/fixtures/secret-grants-before.jsonl")).unwrap();
        let entries: Vec<Entry> = text
            .lines()
            .map(|l| {
                let e: Entry = serde_json::from_str(l).unwrap();
                assert_eq!(serde_json::to_string(&e).unwrap(), l, "read and written back as it was");
                e
            })
            .collect();
        crate::ledger::verify_entries(&entries).unwrap();
        let granted: Vec<&Entry> = entries.iter().filter(|e| e.body.kind == "trust_granted").collect();
        let shape = |e: &Entry| (e.body.data["covers"].as_str().unwrap().to_string(), e.body.data["scope"].as_str().map(String::from), e.body.data["tainted_too"] == json!(true));
        assert_eq!(
            granted.iter().map(|e| shape(e)).collect::<Vec<_>>(),
            [
                ("all-secrets".to_string(), None, false),
                ("secret each".to_string(), None, false),
                ("secret first".to_string(), Some("session".to_string()), false),
                ("secret gh".to_string(), None, true),
                ("secret sys".to_string(), None, true),
                ("all-secrets".to_string(), None, false),
            ],
            "the fixture's grants"
        );
        let standing = unsettled(&entries);
        assert_eq!(standing.iter().map(|g| (g.person.as_str(), g.covers.clone(), g.minutes)).collect::<Vec<_>>(), [("alice", gh(false), 480), ("alice", Covers { name: "sys".into(), system: true }, 45)]);
        // When they were given (the fixture's own time), those two covered their secret and no other.
        let then = at(&granted[3].body.ts) + chrono::Duration::minutes(1);
        assert!(standing[0].covers_use(false, "gh", then) && standing[1].covers_use(true, "sys", then));
        for (system, name) in [(false, "each"), (false, "first"), (false, "std"), (true, "gh"), (false, "sys")] {
            assert!(!standing.iter().any(|g| g.covers_use(system, name, then)), "{name}: covered by none of them");
        }
    }

    /// A session record written before is still read. From before phase 2 of ADR-055: what it said of the
    /// resident agent and of trust is left out. From before 2026-11-16: whether it had read untrusted text is
    /// left out, and one that said it had not is read with `UNSEEN` first, so no program cites it as clean.
    /// Anything else a record does not know is refused, as ever.
    #[test]
    fn a_record_from_before_is_read_and_is_never_clean() {
        let old = r#"{"id":"0123456789abcdef0123456789abcdef","started":"2026-11-10T10:00:00Z","tainted":false,"tainted_by":[],"agent":"torq-agent-casey","proposals":[{"proposal":7,"applied":8},{"proposal":9,"applied":null}],"resident":true,"asks":2,"denied":["user:casey:gh"]}"#;
        let r = parse(old).unwrap();
        assert_eq!(r.tainted_by, [UNSEEN], "never clean");
        assert_eq!(r.proposals, [Cited { proposal: 7 }, Cited { proposal: 9 }]);
        assert_eq!((r.agent.as_str(), r.asks, r.denied.len()), ("torq-agent-casey", 2, 1));
        let tool = r#"{"id":"0123456789abcdef0123456789abcdef","started":"2026-11-10T10:00:00Z","tainted":true,"tainted_by":["not-the-resident-agent","coding-tool"],"agent":"torq-agent-casey","proposals":[],"resident":false,"asks":0,"denied":[]}"#;
        assert_eq!(parse(tool).unwrap().tainted_by, ["not-the-resident-agent", "coding-tool"], "what it said then stays");
        // As the broker wrote one between phase 2 and 2026-11-16: read, and the same as one written now.
        let between = r#"{"id":"0123456789abcdef0123456789abcdef","started":"2026-11-16T10:00:00Z","tainted":true,"tainted_by":["agent-program","read_journal"],"agent":"torq-agent-casey","proposals":[{"proposal":3}],"asks":0,"denied":[]}"#;
        let new = Record { id: "0123456789abcdef0123456789abcdef".into(), started: "2026-11-16T10:00:00Z".into(), tainted_by: vec![UNSEEN.into(), "read_journal".into()], agent: "torq-agent-casey".into(), proposals: vec![Cited { proposal: 3 }], asks: 0, denied: vec![] };
        assert_eq!(parse(between).unwrap(), new);
        let written = serde_json::to_string(&new).unwrap();
        assert!(!written.contains("\"tainted\""), "{written}");
        assert_eq!(parse(&written).unwrap(), new);
        assert!(parse(&old.replace("\"asks\":2", "\"asks\":2,\"trusted\":true")).is_err(), "a field no torq knew is refused");
    }
}
