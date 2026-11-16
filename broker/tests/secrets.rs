//! The values of the secrets (ADR-052): kept encrypted outside the repository, root's alone, the system's and a
//! person's apart, and on the record without the value. systemd-creds is stood in for by a script that takes its
//! arguments and binds each value to its name as the real one does; tests/secrets.nix runs the real one.

// No unsafe code and no dead code, ever (docs/DECISIONS.md ADR-020).
#![forbid(unsafe_code)]
#![forbid(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use torq_broker::secrets::{Granted, Kept, Owner, Used};
use torq_broker::Broker;

const VALUE: &str = "ghp_correct-horse-battery-staple";
/// How long a use waits for its person here: a moment, where a machine waits two minutes.
const WAIT: std::time::Duration = std::time::Duration::from_millis(400);

/// systemd-creds' arguments, and a value bound to its `--name`: one moved to another name does not decrypt.
const CREDS: &str = r#"#!/bin/sh
cmd=$1; shift
name=""; files=""
for a; do case $a in --name=*) name=${a#--name=};; --*) ;; *) files="$files $a";; esac; done
set -- $files
case $cmd in
  encrypt) [ -n "$FAIL" ] && { echo "no TPM" >&2; exit 1; }; rm -f "$2"; umask 022; { printf 'FAKE %s\n' "$name"; base64; } > "$2" ;;
  decrypt) [ "$(head -n 1 "$1")" = "FAKE $name" ] || { echo "name mismatch" >&2; exit 1; }; tail -n +2 "$1" | base64 -d ;;
  *) exit 2 ;;
esac
"#;

struct Machine {
    dir: tempfile::TempDir,
    b: Broker,
}

impl Machine {
    fn new() -> Self {
        Self::with_creds(CREDS)
    }
    fn with_creds(script: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("config");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("desktop.nix"), "{ }\n").unwrap();
        let creds = dir.path().join("creds");
        fs::write(&creds, script).unwrap();
        fs::set_permissions(&creds, fs::Permissions::from_mode(0o755)).unwrap();
        let b = Broker::new(&repo, dir.path().join("state"), "true").with_secrets(dir.path().join("store"), &creds.to_string_lossy()).with_agent_limits(&agents()).with_ask_wait(WAIT);
        b.init().unwrap();
        Machine { dir, b }
    }
    fn store(&self) -> PathBuf {
        self.dir.path().join("store")
    }
    fn ledger(&self) -> String {
        fs::read_to_string(self.b.ledger.path()).unwrap()
    }
    /// The broker as one agent account's request finds it (`--agent`).
    fn as_agent(&self, account: &str) -> Broker {
        Broker::new(self.b.repo.root(), self.dir.path().join("state"), "true").with_agent_limits(&agents()).as_agent(Some(account))
    }
}

/// The broker's map (ADR-051 3): alice's and bob's agents.
fn agents() -> torq_broker::agent::Limits {
    let agents = [("torq-agent-alice", "alice"), ("torq-agent-bob", "bob")].iter().map(|(a, p)| (a.to_string(), p.to_string())).collect();
    torq_broker::agent::Limits { agents, settings: vec![] }
}

/// One use from its start to its end, as the secrets daemon carries it: begun, and while it waits for its person
/// asked about again under its number.
fn used_saying(b: &Broker, u: &torq_broker::secrets::Use, says: &str) -> anyhow::Result<Granted> {
    let mut at = b.begin_use(u, says)?;
    loop {
        match at {
            Used::Granted(g) => return Ok(g),
            Used::Waiting(n) => {
                std::thread::sleep(std::time::Duration::from_millis(20));
                at = b.resume_use(u, n)?;
            }
        }
    }
}

fn used(b: &Broker, u: &torq_broker::secrets::Use) -> anyhow::Result<Granted> {
    used_saying(b, u, "")
}

fn alice() -> Owner {
    Owner::Person("alice".into())
}

#[test]
fn a_value_is_kept_encrypted_private_and_off_the_record() {
    let m = Machine::new();
    let e = m.b.set_secret(&Owner::System, "github-ci", VALUE, "casey").unwrap();
    assert_eq!(e.body.kind, "secret_set");
    assert_eq!((e.body.tier.as_str(), e.body.approved_by.as_deref()), ("system", Some("casey")));
    let file = m.store().join("system/github-ci");
    let kept = fs::read_to_string(&file).unwrap();
    assert!(!kept.contains(VALUE), "kept as systemd-creds made it, never as it was given");
    assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(fs::metadata(m.store().join("system")).unwrap().permissions().mode() & 0o777, 0o700);
    let (shown, value) = m.b.show_secret(&Owner::System, "github-ci", "casey").unwrap();
    assert_eq!((shown.body.kind.as_str(), value.as_str()), ("secret_shown", VALUE));
    m.b.delete_secret(&Owner::System, "github-ci", "casey").unwrap();
    assert!(!file.exists());
    let ledger = m.ledger();
    assert!(!ledger.contains(VALUE), "no value on the record");
    assert!(ledger.contains("secret_set") && ledger.contains("secret_shown") && ledger.contains("secret_deleted"));
}

#[test]
fn a_persons_secret_is_kept_apart_and_bound_to_its_name() {
    let m = Machine::new();
    m.b.set_secret(&alice(), "github", "alice's", "alice").unwrap();
    m.b.set_secret(&Owner::System, "github", "the system's", "casey").unwrap();
    let e = m.b.set_secret(&alice(), "deploy", "key", "alice").unwrap();
    assert_eq!((e.body.tier.as_str(), e.body.data["user"].as_str()), ("user", Some("alice")));
    assert_eq!(m.b.show_secret(&alice(), "github", "alice").unwrap().1, "alice's");
    assert_eq!(m.b.show_secret(&Owner::System, "github", "casey").unwrap().1, "the system's");
    assert_eq!(
        m.b.kept_secrets(),
        vec![
            Kept { owner: Owner::System, name: "github".into() },
            Kept { owner: alice(), name: "deploy".into() },
            Kept { owner: alice(), name: "github".into() },
        ]
    );
    // A value put where another's would be is not that one's: it does not decrypt under another name.
    fs::copy(m.store().join("users/alice/github"), m.store().join("system/github")).unwrap();
    let err = format!("{:#}", m.b.show_secret(&Owner::System, "github", "casey").unwrap_err());
    assert!(err.contains("could not decrypt"), "{err}");
}

#[test]
fn a_new_value_takes_the_old_ones_place() {
    let m = Machine::new();
    m.b.set_secret(&alice(), "t", "one", "alice").unwrap();
    m.b.set_secret(&alice(), "t", "two", "alice").unwrap();
    assert_eq!(m.b.show_secret(&alice(), "t", "alice").unwrap().1, "two");
    let left: Vec<_> = fs::read_dir(m.store().join("users/alice")).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(left, vec![std::ffi::OsString::from("t")], "no part-written file left beside it");
}

#[test]
fn what_is_refused_is_not_recorded() {
    let m = Machine::new();
    let before = m.ledger();
    for bad in ["", "../x", "a/b", "-x", ".x"] {
        assert!(m.b.set_secret(&Owner::System, bad, VALUE, "casey").is_err(), "{bad:?}");
    }
    assert!(m.b.set_secret(&Owner::System, "t", "", "casey").is_err(), "no value");
    assert!(m.b.set_secret(&Owner::System, "t", &"x".repeat(16 * 1024 + 1), "casey").is_err(), "too large");
    assert!(m.b.set_secret(&Owner::System, "t", "a\0b", "casey").is_err(), "a NUL");
    assert!(m.b.delete_secret(&Owner::System, "none", "casey").is_err(), "nothing to delete");
    assert!(m.b.show_secret(&alice(), "none", "alice").is_err(), "nothing to show");
    assert!(Owner::of(Some("../root")).is_err() && Owner::of(Some("Alice")).is_err());
    assert_eq!(m.ledger(), before);
    assert!(m.b.kept_secrets().is_empty());
}

#[test]
fn a_value_not_kept_says_so_and_leaves_nothing() {
    let m = Machine::with_creds(&CREDS.replace("[ -n \"$FAIL\" ]", "true"));
    let err = format!("{:#}", m.b.set_secret(&Owner::System, "t", VALUE, "casey").unwrap_err());
    assert!(err.contains("not kept") && err.contains("no TPM"), "{err}");
    let ledger = m.ledger();
    assert!(ledger.contains("secret_set_failed") && !ledger.contains(VALUE), "{ledger}");
    assert_eq!(fs::read_dir(m.store().join("system")).unwrap().count(), 0, "no part-written file");
    assert!(m.b.kept_secrets().is_empty());
}

/// However the broker is reached (sudo, as well as the socket): a person's value is set and read by them alone.
#[test]
fn a_persons_value_is_theirs_alone_whoever_runs_the_broker() {
    let m = Machine::new();
    m.b.set_secret(&alice(), "gh", VALUE, "alice").unwrap();
    for e in [m.b.set_secret(&alice(), "gh", "another", "casey").map(drop), m.b.show_secret(&alice(), "gh", "casey").map(drop)] {
        let e = format!("{:#}", e.unwrap_err());
        assert!(e.contains("theirs alone"), "{e}");
    }
    assert_eq!(m.b.show_secret(&alice(), "gh", "alice").unwrap().1, VALUE, "unchanged");
    // Deleting it is an administrator's too: the gate (socket.rs) says who may.
    m.b.delete_secret(&alice(), "gh", "casey").unwrap();
}

/// A use (ADR-052, step 2): judged by the rules at HEAD; the agent's first use in each session on the record once,
/// and each refusal of the agent's once; a person's own uses add nothing to it.
#[test]
fn a_use_is_judged_by_its_rule_and_the_agents_first_is_recorded() {
    use torq_broker::secrets::Use;
    let m = Machine::new();
    let repo = m.b.repo.root().to_path_buf();
    fs::create_dir_all(repo.join("users/alice")).unwrap();
    fs::write(
        repo.join("users/alice/secrets.json"),
        r#"{"gh":{"kind":"token","https":{"hosts":["api.github.com"]},"actors":["torq-agent"],"approve":"never-even-tainted"},"asks":{"kind":"token","https":{"hosts":["api.github.com"]},"actors":["torq-agent"],"approve":"never"},"own":{"kind":"token","https":{"hosts":["api.github.com"]}}}"#,
    )
    .unwrap();
    let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&repo).args(args).status().unwrap().success());
    git(&["add", "-A"]);
    git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "rules"]);
    m.b.set_secret(&alice(), "gh", VALUE, "alice").unwrap();
    m.b.set_secret(&alice(), "own", "alice's own", "alice").unwrap();
    m.b.set_secret(&alice(), "asks", VALUE, "alice").unwrap();
    let hers = m.as_agent("torq-agent-alice");
    let session = torq_broker::session::start(&hers).unwrap().id;
    // Alice's agent is her agent account (ADR-051 3): `torq-agent` in her own rules (ADR-052, amended 2026-11-12).
    // What goes without asking here is her standing word for the secret (in the word of before, which is read as that).
    let as_ = |actor: &str, name: &str, host: &str, session: Option<&str>| Use { owner: alice(), name: name.into(), host: host.into(), actor: actor.into(), session: session.map(String::from), ssh: false };
    let agent = |name: &str, host: &str, session: Option<&str>| as_("torq-agent-alice", name, host, session);
    let count = |kind: &str| m.ledger().matches(&format!("\"kind\":\"{kind}\"")).count();
    assert_eq!(used(&m.b, &agent("gh", "api.github.com", Some(&session))).unwrap().value, VALUE);
    assert_eq!(used(&m.b, &agent("gh", "api.github.com", Some(&session))).unwrap().value, VALUE);
    assert_eq!(count("secret_first_use"), 1, "the first use in a session, once");
    let other = torq_broker::session::start(&hers).unwrap().id;
    used(&m.b, &agent("gh", "api.github.com", Some(&other))).unwrap();
    assert_eq!(count("secret_first_use"), 2, "and once in each session");
    for _ in 0..3 {
        assert!(used(&m.b, &agent("gh", "evil.example.com", Some(&session))).is_err());
        assert!(used(&m.b, &agent("own", "api.github.com", Some(&session))).is_err());
    }
    for i in 0..5 {
        assert!(used(&m.b, &agent(&format!("made-up-{i}"), "api.github.com", Some(&session))).is_err());
    }
    assert_eq!(count("secret_use_refused"), 2, "each refusal once, not each time; a name with no rule, never: the daemon's log has it");
    // No one else's agent, and no account that is no one's agent, whatever session it names. Bob's agent's
    // refusal is on the record (an agent's, of a secret with a rule); an account that is no agent's is not.
    let bobs = torq_broker::session::start(&m.as_agent("torq-agent-bob")).unwrap().id;
    // (The old loop's second account, `torq-resident-<person>`, is no one's agent now.)
    for (actor, s) in [("torq-agent-bob", bobs.as_str()), ("torq-agent", session.as_str()), ("torq-resident-alice", session.as_str())] {
        assert!(used(&m.b, &as_(actor, "gh", "api.github.com", Some(s))).is_err(), "{actor}");
    }
    assert_eq!(count("secret_use_refused"), 3, "bob's agent's, once");
    let e = format!("{:#}", used(&m.b, &agent("gh", "api.github.com", Some(&bobs))).unwrap_err());
    assert!(e.contains("is not torq-agent-alice's"), "another agent's session is none of hers to cite: {e}");
    // A secret without her standing word asks a
    // person at every use, whatever its rule said of when in the words of before (here `never`), and no one answers.
    let e = format!("{:#}", used(&m.b, &agent("asks", "api.github.com", Some(&session))).unwrap_err());
    assert!(e.contains("no one answered in time"), "asked, and not answered: {e}");
    let e = format!("{:#}", used(&m.b, &agent("asks", "api.github.com", None)).unwrap_err());
    assert!(e.contains("no one answered in time"), "and a use that names no session the same: {e}");
    assert!(used(&m.b, &agent("gh", "api.github.com", Some("0123456789abcdef0123456789abcdef"))).is_err(), "no such session");
    let before = m.ledger();
    let own = Use { owner: alice(), name: "own".into(), host: "api.github.com".into(), actor: "alice".into(), session: None, ssh: false };
    let g = used(&m.b, &own).unwrap();
    assert_eq!((g.value.as_str(), g.header.as_str(), g.prefix.as_str()), ("alice's own", "Authorization", "Bearer "));
    assert_eq!(m.ledger(), before, "a person's own use: the daemon's log, not the ledger");
    assert!(used(&m.b, &Use { session: Some(session.clone()), ..own }).is_err(), "a person names no session");
    assert!(!m.ledger().contains(VALUE));
}

/// An agent socket's grant (ADR-052, SSH): who it is for, for an hour; a session only the agent's, and one the
/// broker holds.
#[test]
fn an_agent_sockets_grant_is_who_asked() {
    let m = Machine::new();
    let g = m.b.grant_agent("alice", 1000, None).unwrap();
    let read = m.b.grant(&g).unwrap();
    assert_eq!((read.actor.as_str(), read.uid, read.session), ("alice", 1000, None));
    assert!(m.b.grant_agent("alice", 1000, Some("0123456789abcdef0123456789abcdef")).is_err(), "a person names no session");
    assert!(m.b.grant_agent("torq-agent-alice", 990, Some("0123456789abcdef0123456789abcdef")).is_err(), "no such session");
    let s = torq_broker::session::start(&m.as_agent("torq-agent-alice")).unwrap().id;
    let a = m.b.grant_agent("torq-agent-alice", 990, Some(&s)).unwrap();
    assert_eq!(m.b.grant(&a).unwrap().session.as_deref(), Some(s.as_str()));
    assert!(m.b.grant_agent("torq-agent-bob", 991, Some(&s)).is_err(), "another agent's session");
    assert!(m.b.grant_agent("torq-agent", 989, Some(&s)).is_err(), "an account that is no one's agent names no session");
    assert!(m.b.grant_agent("torq-resident-alice", 980, Some(&s)).is_err(), "the old loop's second account is no agent");
    for bad in ["", "../x", "0123456789abcdef0123456789abcdeg", "00000000000000000000000000000000"] {
        assert!(m.b.grant(bad).is_err(), "{bad:?}");
    }
    assert!(m.b.ssh_keys_for("alice").is_empty(), "no keys yet");
}

/// An SSH key's public half, read from its own header (no program given the private key, no file of it), beside it;
/// a key under a passphrase, or another form, refused saying why, and the old one left listed.
#[test]
fn an_ssh_keys_public_half_is_kept_beside_it() {
    const KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\nQyNTUxOQAAACALEJlaQgiCLSgzMKjXIAECACR/bjD9FbMY71yZLrLm3AAAAIjyZMAb8mTA\n";
    let m = Machine::new();
    let Some(key) = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../wire/src/secrets.rs")).ok().and_then(|w| {
        let start = w.find("const TEST_KEY: &str = \"")? + "const TEST_KEY: &str = \"".len();
        Some(w[start..start + w[start..].find('"')?].replace("\\n", "\n"))
    }) else {
        panic!("the test key")
    };
    assert!(key.starts_with(KEY), "the same key as torq-wire's test");
    m.b.set_secret(&alice(), "deploy", &key, "alice").unwrap();
    let public = fs::read_to_string(m.store().join("users/alice/deploy.pub")).unwrap();
    assert_eq!(public.trim(), "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAsQmVpCCIItKDMwqNcgAQIAJH9uMP0VsxjvXJkusubc");
    let left: Vec<String> = fs::read_dir(m.store().join("users/alice")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert!(left.iter().all(|n| n == "deploy" || n == "deploy.pub"), "nothing else, no key file of any kind: {left:?}");
    let e = format!("{:#}", m.b.set_secret(&alice(), "deploy", "-----BEGIN RSA PRIVATE KEY-----\nMIIE\n-----END RSA PRIVATE KEY-----", "alice").unwrap_err());
    assert!(e.contains("PEM"), "{e}");
    assert_eq!(fs::read_to_string(m.store().join("users/alice/deploy.pub")).unwrap(), public, "the old one still listed");
    m.b.set_secret(&alice(), "deploy", "now a token", "alice").unwrap();
    assert!(!m.store().join("users/alice/deploy.pub").exists(), "not a key now: no public half");
}

// ── Grants, and uses that wait for a person (ADR-051 and ADR-052, amended 2026-11-13) ─────────────────────────────────

use torq_broker::ask::Ask;
use torq_broker::secrets::Use;
use torq_broker::session::{self, Covers, Ending};

/// Long enough that a test's own answer always comes first.
const LONG: std::time::Duration = std::time::Duration::from_secs(30);

/// A broker on `dir`'s machine, as a request of its own finds it: the one a use waits in, beside the test's.
fn broker_at(dir: &std::path::Path, wait: std::time::Duration) -> Broker {
    Broker::new(dir.join("config"), dir.join("state"), "true").with_secrets(dir.join("store"), &dir.join("creds").to_string_lossy()).with_agent_limits(&agents()).with_ask_wait(wait)
}

/// A machine with rules and values: alice's own (`gh` never asks, `each` each use, `first` the first, `std`
/// never-even-tainted, `own` not the agent's; in the words rules had before 2026-11-16, which are read as what they did), bob's own `gh`, and the system's (`sys` for both agents, `bobs` for
/// bob's alone), every one a token for api.github.com. And two sessions of alice's agent.
struct Asked {
    m: Machine,
    tool: String,
    second: String,
}

impl Asked {
    fn new() -> Self {
        let m = Machine::new();
        let repo = m.b.repo.root().to_path_buf();
        let token = |extra: &str| format!(r#"{{"kind":"token","https":{{"hosts":["api.github.com"]}}{extra}}}"#);
        let agents = |approve: &str| token(&format!(r#","actors":["torq-agent"],"approve":"{approve}""#));
        fs::create_dir_all(repo.join("users/alice")).unwrap();
        fs::create_dir_all(repo.join("users/bob")).unwrap();
        fs::create_dir_all(repo.join("secrets")).unwrap();
        fs::write(repo.join("users/alice/secrets.json"), format!(r#"{{"gh":{},"each":{},"first":{},"std":{},"own":{}}}"#, agents("never"), agents("each-use"), agents("first-use"), agents("never-even-tainted"), token(""))).unwrap();
        fs::write(repo.join("users/bob/secrets.json"), format!(r#"{{"gh":{}}}"#, agents("each-use"))).unwrap();
        fs::write(repo.join("secrets/system.json"), format!(r#"{{"sys":{},"bobs":{},"each":{}}}"#, token(r#","actors":["torq-agent-alice","torq-agent-bob"],"approve":"each-use""#), token(r#","actors":["torq-agent-bob"],"approve":"each-use""#), token(r#","actors":["torq-agent-alice"],"approve":"each-use""#))).unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&repo).args(args).status().unwrap().success());
        git(&["add", "-A"]);
        git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "rules"]);
        for name in ["gh", "each", "first", "std", "own"] {
            m.b.set_secret(&alice(), name, VALUE, "alice").unwrap();
        }
        m.b.set_secret(&Owner::Person("bob".into()), "gh", VALUE, "bob").unwrap();
        m.b.set_secret(&Owner::System, "sys", VALUE, "casey").unwrap();
        m.b.set_secret(&Owner::System, "bobs", VALUE, "casey").unwrap();
        // A system secret of the same name as one of alice's own: the two are never taken for each other.
        m.b.set_secret(&Owner::System, "each", VALUE, "casey").unwrap();
        let tool = session::start(&m.as_agent("torq-agent-alice")).unwrap().id;
        let second = session::start(&m.as_agent("torq-agent-alice")).unwrap().id;
        Asked { m, tool, second }
    }

    /// Her agent's use of alice's secret `name`, from its session.
    fn tool(&self, name: &str) -> Use {
        Use { owner: alice(), name: name.into(), host: "api.github.com".into(), actor: "torq-agent-alice".into(), session: Some(self.tool.clone()), ssh: false }
    }

    /// The same, from its other session.
    fn second(&self, name: &str) -> Use {
        Use { session: Some(self.second.clone()), ..self.tool(name) }
    }

    /// `u`, begun in a request of its own that waits up to `wait`: its value, or why not.
    fn begin(&self, u: Use, says: &str, wait: std::time::Duration) -> std::thread::JoinHandle<Result<String, String>> {
        let (dir, says) = (self.m.dir.path().to_path_buf(), says.to_string());
        std::thread::spawn(move || used_saying(&broker_at(&dir, wait), &u, &says).map(|g| g.value).map_err(|e| format!("{e:#}")))
    }

    /// The uses that wait for `person`, once there are `count` of them.
    fn waiting(&self, person: &str, count: usize) -> Vec<Ask> {
        let t = std::time::Instant::now();
        loop {
            let w = self.m.b.waiting_asks(person).unwrap();
            if w.len() == count {
                return w;
            }
            assert!(t.elapsed() < std::time::Duration::from_secs(20), "{} wait for {person}, not {count}", w.len());
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    fn count(&self, kind: &str) -> usize {
        self.m.ledger().matches(&format!("\"kind\":\"{kind}\"")).count()
    }

    /// The ledger's entries of `kind`, as JSON.
    fn entries(&self, kind: &str) -> Vec<serde_json::Value> {
        self.m.ledger().lines().map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()).filter(|e| e["kind"] == kind).collect()
    }

    /// A use that asks and is not answered: refused at its time (the machine's short wait).
    fn unanswered(&self, u: &Use) -> String {
        format!("{:#}", used(&self.m.b, u).unwrap_err())
    }
}

/// A use that asks waits for the person its agent serves; their yes lets it through, once, and the record has the
/// one entry: who asked, of what, for which host, in which session, who answered and where. Never the value.
#[test]
fn a_use_that_asks_waits_and_the_persons_yes_lets_it_through() {
    let a = Asked::new();
    let waiter = a.begin(a.tool("each"), "deploying the site", LONG);
    let w = a.waiting("alice", 1);
    let f = &w[0].facts;
    assert_eq!((f.person.as_str(), f.agent.as_str(), f.caller.as_str(), f.secret.as_str(), f.host.as_str(), f.ssh), ("alice", "torq-agent-alice", "torq-agent-alice", "each", "api.github.com", false));
    assert_eq!((f.owner.clone(), f.session.as_deref(), w[0].agent_says.as_str()), (alice(), Some(a.tool.as_str()), "deploying the site"));
    assert!(a.m.b.waiting_asks("bob").unwrap().is_empty(), "asked of alice, and of no one else");
    let e = a.m.b.answer_ask(w[0].n, "alice", true, false, "terminal").unwrap();
    assert_eq!(waiter.join().unwrap().unwrap(), VALUE);
    assert_eq!((e.body.kind.as_str(), e.body.approved_by.as_deref(), e.body.actor.as_str()), ("secret_use_approved", Some("alice"), "user"));
    let d = &e.body.data;
    assert_eq!((d["secret"].as_str(), d["user"].as_str(), d["host"].as_str(), d["session"].as_str(), d["by"].as_str(), d["where"].as_str()), (Some("each"), Some("alice"), Some("api.github.com"), Some(a.tool.as_str()), Some("alice"), Some("terminal")));
    assert_eq!((d["agent"].as_str(), d["asked_by"].as_str(), d["ask"].as_u64()), (Some("torq-agent-alice"), Some("torq-agent-alice"), Some(w[0].n)));
    assert_eq!((a.count("secret_use_approved"), a.count("secret_use_denied"), a.count("secret_use_timed_out")), (1, 0, 0));
    assert!(a.m.b.waiting_asks("alice").unwrap().is_empty(), "answered: it waits no more");
    // Once: the next use of an each-use secret asks again.
    assert!(a.unanswered(&a.tool("each")).contains("no one answered"));
    assert!(!a.m.ledger().contains(VALUE) && !a.m.ledger().contains("deploying the site"), "no value on the record, nor the agent's words");
}

/// Their no refuses it; and a use no one answers is refused at its time. Each is on the record once, as what it was.
#[test]
fn a_no_refuses_and_so_does_no_answer() {
    let a = Asked::new();
    let waiter = a.begin(a.tool("each"), "", LONG);
    let n = a.waiting("alice", 1)[0].n;
    let e = a.m.b.answer_ask(n, "alice", false, false, "dialog").unwrap();
    assert_eq!((e.body.kind.as_str(), e.body.data["where"].as_str(), e.body.data.get("for")), ("secret_use_denied", Some("dialog"), None));
    let why = waiter.join().unwrap().unwrap_err();
    assert!(why.contains("alice denied"), "{why}");
    let t = std::time::Instant::now();
    let why = a.unanswered(&a.tool("each"));
    assert!(why.contains("no one answered"), "{why}");
    assert!(t.elapsed() >= WAIT && t.elapsed() < WAIT * 10, "it waited its time, and no longer: {:?}", t.elapsed());
    assert_eq!((a.count("secret_use_approved"), a.count("secret_use_denied"), a.count("secret_use_timed_out")), (0, 1, 1));
    let out = &a.entries("secret_use_timed_out")[0];
    assert_eq!((out["actor"].as_str(), out["approved_by"].as_str(), out["data"]["secret"].as_str()), (Some("broker"), None, Some("each")));
    // Its time over, it has no answer: not a yes that comes late.
    let last = out["data"]["ask"].as_u64().unwrap();
    let late = format!("{:#}", a.m.b.answer_ask(last, "alice", true, false, "terminal").unwrap_err());
    assert!(late.contains("no use waits"), "{late}");
    assert_eq!(a.count("secret_use_approved"), 0);
    assert!(a.m.b.answer_ask(n, "alice", true, false, "window").is_err(), "an answer is given in the dialog or at a terminal");
}

/// Only the person the agent serves answers: not the agent's own account, not another person, not an
/// administrator, not root. And the first answer stands.
#[test]
fn only_the_agents_person_answers_and_the_first_answer_stands() {
    let a = Asked::new();
    let waiter = a.begin(a.tool("gh"), "", LONG);
    let n = a.waiting("alice", 1)[0].n;
    for other in ["torq-agent-alice", "torq-resident-alice", "bob", "casey", "root", "torq-secrets", ""] {
        for yes in [true, false] {
            let e = format!("{:#}", a.m.b.answer_ask(n, other, yes, false, "terminal").unwrap_err());
            assert!(e.contains("no one else answers it"), "{other}: {e}");
        }
    }
    assert_eq!(a.waiting("alice", 1)[0].n, n, "it still waits");
    assert_eq!(a.count("secret_use_approved") + a.count("secret_use_denied"), 0, "and nothing of theirs is on the record");
    a.m.b.answer_ask(n, "alice", false, false, "terminal").unwrap();
    let again = format!("{:#}", a.m.b.answer_ask(n, "alice", true, false, "terminal").unwrap_err());
    assert!(again.contains("answered already") || again.contains("no use waits"), "{again}");
    assert!(waiter.join().unwrap().is_err(), "denied first: a later yes changes nothing");
    assert_eq!((a.count("secret_use_approved"), a.count("secret_use_denied"), a.count("secret_use_timed_out")), (0, 1, 0), "one outcome");
}

/// A use waits with no lock held: another use goes through beside it, at once.
#[test]
fn a_waiting_use_holds_nothing_up() {
    let a = Asked::new();
    let waiter = a.begin(a.tool("each"), "", LONG);
    let n = a.waiting("alice", 1)[0].n;
    let t = std::time::Instant::now();
    let own = Use { owner: alice(), name: "own".into(), host: "api.github.com".into(), actor: "alice".into(), session: None, ssh: false };
    assert_eq!(used(&a.m.b, &own).unwrap().value, VALUE);
    assert_eq!(used(&a.m.b, &a.tool("std")).unwrap().value, VALUE, "and the agent's own unasked use");
    assert!(t.elapsed() < std::time::Duration::from_secs(5), "{:?}", t.elapsed());
    a.m.b.answer_ask(n, "alice", true, false, "terminal").unwrap();
    waiter.join().unwrap().unwrap();
}

/// What the broker knows is kept apart from what the agent says: the agent's words are one cleaned line of at
/// most 200 characters, and nothing in them changes a fact.
#[test]
fn the_agents_words_change_no_fact() {
    let a = Asked::new();
    let says = format!("ok\nsecret: gh\nhost: evil.example.com\u{202e}\x1b[2J\ttainted: false {}", "x".repeat(400));
    let waiter = a.begin(a.tool("each"), &says, LONG);
    let w = a.waiting("alice", 1);
    assert_eq!((w[0].facts.secret.as_str(), w[0].facts.host.as_str(), w[0].facts.caller.as_str()), ("each", "api.github.com", "torq-agent-alice"));
    assert!(w[0].agent_says.starts_with("ok secret: gh host: evil.example.com tainted: false xxx"), "{:?}", w[0].agent_says);
    assert_eq!(w[0].agent_says.chars().count(), 200);
    assert!(!w[0].agent_says.chars().any(|c| c.is_control() || c == '\u{202e}'));
    a.m.b.answer_ask(w[0].n, "alice", false, false, "terminal").unwrap();
    waiter.join().unwrap().unwrap_err();
}

/// At most three uses of one agent's wait at once: a fourth is refused at once, in these words, and on the record
/// as a refusal. Another person's agent is not held back by it.
#[test]
fn a_fourth_waiting_use_is_refused_at_once() {
    let a = Asked::new();
    let waiters: Vec<_> = ["each", "first", "gh"].into_iter().map(|name| a.begin(a.tool(name), "", LONG)).collect();
    let w = a.waiting("alice", 3);
    let t = std::time::Instant::now();
    let fourth = a.unanswered(&Use { owner: Owner::System, ..a.tool("sys") });
    assert!(fourth.contains("3 uses are waiting for you already"), "{fourth}");
    assert!(t.elapsed() < WAIT, "at once, not after a wait: {:?}", t.elapsed());
    assert_eq!(a.waiting("alice", 3).len(), 3, "and it does not wait");
    assert_eq!(a.entries("secret_use_refused").iter().filter(|e| e["data"]["error"].as_str().is_some_and(|s| s.contains("3 uses are waiting"))).count(), 1);
    // Bob's agent asks bob all the same.
    let bobs = session::start(&a.m.as_agent("torq-agent-bob")).unwrap().id;
    let bob = a.begin(Use { owner: Owner::Person("bob".into()), name: "gh".into(), host: "api.github.com".into(), actor: "torq-agent-bob".into(), session: Some(bobs), ssh: false }, "", LONG);
    let b = a.waiting("bob", 1);
    a.m.b.answer_ask(b[0].n, "bob", true, false, "terminal").unwrap();
    bob.join().unwrap().unwrap();
    // One answered, and there is room for one more.
    a.m.b.answer_ask(w[0].n, "alice", false, false, "terminal").unwrap();
    assert!(a.unanswered(&Use { owner: Owner::System, ..a.tool("sys") }).contains("no one answered"), "room again: it waits its time");
    for ask in &w[1..] {
        a.m.b.answer_ask(ask.n, "alice", false, false, "terminal").unwrap();
    }
    for h in waiters {
        h.join().unwrap().unwrap_err();
    }
}

/// A person is asked at most ten times about one session: the eleventh use that would ask is refused at once, and
/// says so. Another session of the agent's asks again.
#[test]
fn a_session_asks_at_most_ten_times() {
    let a = Asked::new();
    for i in 0..10 {
        assert!(a.unanswered(&a.tool("each")).contains("no one answered"), "{i}");
    }
    assert_eq!(a.count("secret_use_timed_out"), 10);
    let t = std::time::Instant::now();
    let more = a.unanswered(&a.tool("each"));
    assert!(more.contains("asked 10 times") && more.contains("no more are asked"), "{more}");
    assert!(t.elapsed() < WAIT, "at once: {:?}", t.elapsed());
    assert!(a.unanswered(&a.tool("first")).contains("asked 10 times"), "any secret, for the rest of the session");
    assert_eq!(a.count("secret_use_timed_out"), 10, "no one was asked");
    assert_eq!(used(&a.m.b, &a.tool("std")).unwrap().value, VALUE, "what asks no one still goes");
    assert!(a.unanswered(&a.second("each")).contains("no one answered"), "another session is asked");
}

/// "Deny, and don't ask again this session": that secret's uses in that session are refused at once from then on,
/// and the record says `denied` with `for: session`. Another secret, and another session, still ask.
#[test]
fn a_denial_for_the_rest_of_the_session_stops_the_asking() {
    let a = Asked::new();
    let waiter = a.begin(a.tool("each"), "", LONG);
    let n = a.waiting("alice", 1)[0].n;
    assert!(a.m.b.answer_ask(n, "alice", true, true, "terminal").is_err(), "a yes is never for the rest of a session: that is a grant");
    let e = a.m.b.answer_ask(n, "alice", false, true, "dialog").unwrap();
    assert_eq!((e.body.kind.as_str(), e.body.data["for"].as_str()), ("secret_use_denied", Some("session")));
    waiter.join().unwrap().unwrap_err();
    let t = std::time::Instant::now();
    let again = a.unanswered(&a.tool("each"));
    assert!(again.contains("not to be asked again in this session"), "{again}");
    assert!(t.elapsed() < WAIT && a.m.b.waiting_asks("alice").unwrap().is_empty(), "refused at once, and no one is asked");
    assert_eq!(a.count("secret_use_timed_out"), 0);
    assert!(a.unanswered(&a.tool("first")).contains("no one answered"), "another secret asks");
    assert!(a.unanswered(&a.second("each")).contains("no one answered"), "another session asks");
}

/// What was left behind is no waiting use: a file whose time is over (its waiter died, the machine restarted) is
/// ended as timed out, on the record once; one whose times are not a use's is not answered; and a number is never
/// given twice, whatever was left.
#[test]
fn a_use_left_behind_is_ended_once_and_its_number_not_given_again() {
    let a = Asked::new();
    assert!(a.unanswered(&a.tool("each")).contains("no one answered"));
    let first = a.entries("secret_use_timed_out")[0]["data"]["ask"].as_u64().unwrap();
    let asks = a.m.dir.path().join("state/secret-asks");
    assert_eq!(fs::metadata(&asks).unwrap().permissions().mode() & 0o777, 0o700, "root's alone");
    // (As the broker of before 2026-11-16 kept one, with the two things it said more: still read, and ended.)
    let facts = serde_json::json!({ "person": "alice", "agent": "torq-agent-alice", "caller": "torq-agent-alice", "owner": { "person": "alice" }, "secret": "each", "host": "api.github.com", "ssh": false, "session": a.tool, "tainted": true, "why": "left behind" });
    let left = |n: u64, asked: chrono::DateTime<chrono::Utc>, until: chrono::DateTime<chrono::Utc>| {
        fs::write(asks.join(format!("{n}.json")), serde_json::json!({ "n": n, "asked": asked.to_rfc3339(), "until": until.to_rfc3339(), "facts": facts, "agent_says": "", "answer": null }).to_string()).unwrap();
    };
    let now = chrono::Utc::now();
    let minutes = chrono::Duration::minutes;
    left(40, now - minutes(10), now - minutes(8));
    // Not a use's times: two minutes at most, and not asked in the future.
    left(41, now - minutes(1), now + minutes(30));
    left(42, now + minutes(5), now + minutes(6));
    for n in [40, 41, 42] {
        let e = format!("{:#}", a.m.b.answer_ask(n, "alice", true, false, "terminal").unwrap_err());
        assert!(e.contains("no use waits"), "{n}: {e}");
    }
    assert!(a.m.b.waiting_asks("alice").unwrap().is_empty());
    assert_eq!((a.count("secret_use_timed_out"), a.count("secret_use_approved")), (4, 0), "each ended once, as timed out; none approved");
    a.m.b.waiting_asks("alice").unwrap();
    assert_eq!(a.count("secret_use_timed_out"), 4, "and not again");
    // The next number is past every one given or found, even with the count lost.
    fs::remove_file(asks.join("next")).unwrap();
    left(77, now - minutes(10), now - minutes(8));
    let waiter = a.begin(a.tool("each"), "", LONG);
    let w = a.waiting("alice", 1);
    assert!(w[0].n > 77 && w[0].n > first, "{}", w[0].n);
    assert_eq!(fs::metadata(asks.join(format!("{}.json", w[0].n))).unwrap().permissions().mode() & 0o777, 0o600);
    a.m.b.answer_ask(w[0].n, "alice", true, false, "terminal").unwrap();
    waiter.join().unwrap().unwrap();
}

/// What a person approved is the use they were shown: a use whose rule was taken away while it waited is refused
/// though approved. (What its session reports reading meanwhile changes nothing a person was shown.)
#[test]
fn what_changed_while_it_waited_is_not_what_was_approved() {
    let a = Asked::new();
    let waiter = a.begin(a.tool("each"), "", LONG);
    let n = a.waiting("alice", 1)[0].n;
    session::taint(&a.m.as_agent("torq-agent-alice"), &a.tool, "web").unwrap();
    a.m.b.answer_ask(n, "alice", true, false, "terminal").unwrap();
    assert_eq!(waiter.join().unwrap().unwrap(), VALUE, "as it was shown");
    let waiter = a.begin(a.tool("each"), "", LONG);
    let n = a.waiting("alice", 1)[0].n;
    let repo = a.m.b.repo.root().to_path_buf();
    fs::write(repo.join("users/alice/secrets.json"), r#"{"each":{"kind":"token","https":{"hosts":["api.github.com"]}}}"#).unwrap();
    assert!(std::process::Command::new("git").arg("-C").arg(&repo).args(["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qam", "not the agent's"]).status().unwrap().success());
    a.m.b.answer_ask(n, "alice", true, false, "terminal").unwrap();
    let why = waiter.join().unwrap().unwrap_err();
    assert!(why.contains("may not use"), "{why}");
}

/// A grant lets its one secret's uses go without asking, for its time, and each first use under it names it; taken
/// back, the asking is back at once. Given by a person for their own agent; one secret, for 1 to 480 minutes.
#[test]
fn a_grant_lets_its_secrets_uses_through_until_it_ends() {
    let a = Asked::new();
    let each = || Covers { name: "each".into(), system: false };
    for bad in [0, 481, 100_000] {
        assert!(session::grant_secret(&a.m.b, "alice", each(), bad, false).is_err(), "{bad} minutes");
    }
    assert!(session::grant_secret(&a.m.b, "carol", each(), 5, false).is_err(), "a person with no agent");
    assert!(session::grant_secret(&a.m.b, "alice", Covers { name: "none".into(), system: false }, 5, false).is_err(), "a secret with no rule");
    // `allow all for <minutes>` once let the agent use every secret: someone who learned it is told it is gone,
    // and what to type, whether they meant their own secrets or the system's.
    for system in [false, true] {
        let e = format!("{:#}", session::grant_secret(&a.m.b, "alice", Covers { name: "all".into(), system }, 30, false).unwrap_err());
        assert_eq!(e, "refused: allowing every secret at once is gone. Name the one secret: request system secret allow <name> for <minutes> (show system secrets lists them)");
    }
    assert!(a.unanswered(&a.tool("each")).contains("no one answered"), "no grant: asked");
    let g = session::grant_secret(&a.m.b, "alice", each(), 480, false).unwrap();
    assert_eq!(g.body.reason, "alice lets the agent use their secret each without asking, for 480 minutes");
    assert_eq!((g.body.kind.as_str(), g.body.approved_by.as_deref(), g.body.data["covers"].as_str(), g.body.data["owner"].as_str(), g.body.data["minutes"].as_u64()), ("trust_granted", Some("alice"), Some("secret each"), Some("user"), Some(480)));
    assert_eq!(g.body.data["tainted_too"], serde_json::json!(true), "the ledger's word for the one kind of grant, as before");
    assert!(session::grant_secret(&a.m.b, "alice", each(), 5, false).is_err(), "one at a time");
    assert_eq!(used(&a.m.b, &a.tool("each")).unwrap().value, VALUE);
    assert_eq!(used(&a.m.b, &a.tool("each")).unwrap().value, VALUE);
    let first = a.entries("secret_first_use");
    assert_eq!((first.len(), first[0]["data"]["grant"].as_u64()), (1, Some(g.body.seq)), "its use names the grant it went under");
    assert_eq!(used(&a.m.b, &Use { session: None, ..a.tool("each") }).unwrap().value, VALUE, "and a use that names no session");
    assert_eq!((a.count("secret_use_approved"), a.count("secret_use_timed_out")), (0, 1), "no one was asked but before the grant");
    assert!(a.unanswered(&a.tool("first")).contains("no one answered"), "another secret is not covered");
    let ended = session::revoke_secrets(&a.m.b, "alice", &Ending::Secret { name: "each".into(), system: false }).unwrap();
    assert_eq!((ended.len(), ended[0].body.kind.as_str(), ended[0].body.data["grant"].as_u64(), ended[0].body.data["covers"].as_str()), (1, "trust_revoked", Some(g.body.seq), Some("secret each")));
    assert!(a.unanswered(&a.tool("each")).contains("no one answered"), "taken back: it asks again");
    assert!(session::revoke_secrets(&a.m.b, "alice", &Ending::Everything).unwrap().is_empty(), "nothing left to end");
}

/// A grant made `replace`ing ends the one that holds of the same secret in the same step, and is the only one left;
/// one refused before it is made (a bad time, a secret with no rule) leaves the old one as it was. Without `replace`
/// a second is still refused.
#[test]
fn a_grant_that_replaces_ends_the_old_one_only_when_it_is_made() {
    let a = Asked::new();
    let each = || Covers { name: "each".into(), system: false };
    let live = |a: &Asked| session::active(&a.m.b.ledger.read_all().unwrap()).iter().map(|g| (g.seq, g.minutes)).collect::<Vec<_>>();
    let old = session::grant_secret(&a.m.b, "alice", each(), 5, false).unwrap();
    assert!(session::grant_secret(&a.m.b, "alice", each(), 60, false).is_err(), "still one at a time");
    assert!(session::grant_secret(&a.m.b, "alice", each(), 0, true).is_err());
    assert!(session::grant_secret(&a.m.b, "alice", each(), 481, true).is_err());
    assert_eq!(live(&a), [(old.body.seq, 5)], "refused before it was made: the old one stands");
    let new = session::grant_secret(&a.m.b, "alice", each(), 60, true).unwrap();
    assert_eq!(live(&a), [(new.body.seq, 60)]);
    assert_eq!(a.entries("trust_revoked")[0]["data"]["grant"].as_u64(), Some(old.body.seq), "the old one is on the record as ended");
    // With nothing to replace it is a plain grant.
    session::revoke_secrets(&a.m.b, "alice", &Ending::Everything).unwrap();
    session::grant_secret(&a.m.b, "alice", each(), 5, true).unwrap();
    assert_eq!(live(&a).len(), 1);
}

/// Ending a grant ends that one: a person's own secret and a system secret of the same name are told apart, as
/// the grant told them apart; and everything is everything of theirs, and nothing of anyone else's.
#[test]
fn ending_a_grant_ends_that_one_and_no_other() {
    let a = Asked::new();
    let named = |system: bool| Covers { name: "each".into(), system };
    let covers = |a: &Asked| session::active(&a.m.b.ledger.read_all().unwrap()).into_iter().filter(|g| g.person == "alice").map(|g| g.covers).collect::<Vec<_>>();
    let give = |a: &Asked| {
        for c in [named(false), named(true)] {
            if !covers(a).contains(&c) {
                session::grant_secret(&a.m.b, "alice", c, 30, false).unwrap();
            }
        }
    };
    session::grant_secret(&a.m.b, "bob", Covers { name: "gh".into(), system: false }, 30, false).unwrap();
    give(&a);
    assert_eq!(covers(&a), [named(false), named(true)]);
    // Her own secret of that name: the system's grant stays.
    let ended = session::revoke_secrets(&a.m.b, "alice", &Ending::Secret { name: "each".into(), system: false }).unwrap();
    assert_eq!((ended.len(), covers(&a)), (1, vec![named(true)]));
    // The system's of that name: her own stays.
    give(&a);
    let ended = session::revoke_secrets(&a.m.b, "alice", &Ending::Secret { name: "each".into(), system: true }).unwrap();
    assert_eq!((ended.len(), covers(&a)), (1, vec![named(false)]));
    // Everything: all of hers, and nothing of anyone else's.
    give(&a);
    assert_eq!(session::revoke_secrets(&a.m.b, "alice", &Ending::Everything).unwrap().len(), 2);
    assert_eq!(session::active(&a.m.b.ledger.read_all().unwrap()).iter().map(|g| g.person.as_str()).collect::<Vec<_>>(), ["bob"]);
}

/// What a grant never covers (ADR-051 and ADR-052 4, amended 2026-11-13 and 2026-11-16), each line of it.
#[test]
fn what_a_grant_never_covers() {
    let a = Asked::new();
    let named = |name: &str, system: bool| Covers { name: name.into(), system };
    // Whose secret a grant is of is said in its entry, which the terminal prints and the dialog words alike.
    let g = session::grant_secret(&a.m.b, "alice", named("sys", true), 480, false).unwrap();
    assert!(g.body.reason.contains("the system's secret sys") && g.body.data["owner"] == "system", "{}", g.body.reason);
    let system = |name: &str, u: Use| Use { owner: Owner::System, name: name.into(), ..u };
    // A system secret whose rule does not name this agent: no grant is given of it, and none widens its rule.
    let e = format!("{:#}", session::grant_secret(&a.m.b, "alice", named("bobs", true), 5, false).unwrap_err());
    assert!(e.contains("widens no rule"), "{e}");
    assert!(a.unanswered(&system("bobs", a.tool("x"))).contains("may not use"));
    assert_eq!(used(&a.m.b, &system("sys", a.tool("x"))).unwrap().value, VALUE, "one whose rule names it, and its grant");
    // A secret its rule gives to no agent: no grant is given of it, and it is still not the agent's.
    assert!(a.unanswered(&a.tool("own")).contains("may not use"));
    assert!(session::grant_secret(&a.m.b, "alice", named("own", false), 5, false).is_err());
    // Another person's secret, or another person's agent: alice's grants are of her agent's uses alone.
    let bobs = session::start(&a.m.as_agent("torq-agent-bob")).unwrap().id;
    let bob = |u: Use| Use { actor: "torq-agent-bob".into(), session: Some(bobs.clone()), ..u };
    assert!(a.unanswered(&bob(system("sys", a.tool("x")))).contains("no one answered"), "bob's agent, of a system secret alice let hers use: bob is asked");
    assert!(a.unanswered(&bob(a.tool("each"))).contains("may not use"), "bob's agent, of alice's secret");
    assert!(a.unanswered(&Use { owner: Owner::Person("bob".into()), ..a.tool("gh") }).contains("may not use"), "alice's agent, of bob's secret");
    // A host or an operation the rule does not name.
    assert!(a.unanswered(&Use { host: "evil.example.com".into(), ..a.tool("each") }).contains("is not sent to"));
    assert!(a.unanswered(&Use { ssh: true, ..a.tool("each") }).contains("signs for nothing"));
    // Reading a value back, setting one, deleting one: a grant is of uses.
    assert!(format!("{:#}", a.m.b.show_secret(&alice(), "each", "torq-agent-alice").unwrap_err()).contains("theirs alone"));
    assert!(format!("{:#}", a.m.b.set_secret(&alice(), "each", "another", "torq-agent-alice").unwrap_err()).contains("theirs alone"));
    // Any other secret: her own of another name, a system secret of the same name as one of hers, with a
    // session or without. Each is asked, and says why.
    let why = a.unanswered(&a.tool("each"));
    assert!(why.contains("no one answered in time (TorqOS cannot see what an agent has read, so its person approves each use of a secret, unless they said otherwise for that secret)"), "{why}");
    assert!(!why.contains("taint") && !why.contains("session"), "{why}");
    assert!(a.unanswered(&system("each", a.tool("x"))).contains("no one answered"), "a system secret with no grant of its own");
    assert!(a.unanswered(&Use { session: None, ..a.tool("each") }).contains("no one answered"), "a use that names no session");
    let g = session::grant_secret(&a.m.b, "alice", named("each", false), 5, false).unwrap();
    assert_eq!(used(&a.m.b, &a.tool("each")).unwrap().value, VALUE, "the one named secret");
    assert_eq!(a.entries("secret_first_use").last().unwrap()["data"]["grant"].as_u64(), Some(g.body.seq));
    assert!(a.unanswered(&a.tool("first")).contains("no one answered"), "and no other");
    assert_eq!(a.count("secret_use_approved"), 0);
}

/// When a person is asked, by the rule's word (ADR-052 4, amended 2026-11-16). This machine's rules are written
/// in the words of before (the fixture of `Asked`): each is read as what it did then. `each-use`, `first-use`
/// and `never` ask at every use of an agent's, in every one of its sessions and with none: a person's yes is
/// for the one use. Only the person's own standing word for one secret (`never-even-tainted` then,
/// `without-asking` now) lets a use through with no one asked.
#[test]
fn every_use_is_asked_but_for_the_persons_standing_word() {
    let a = Asked::new();
    for (name, word) in [("each", "each-use"), ("first", "first-use"), ("gh", "never")] {
        assert!(a.unanswered(&a.tool(name)).contains("no one answered"), "{word}");
        assert!(a.unanswered(&a.second(name)).contains("no one answered"), "{word}, in another session");
        assert!(a.unanswered(&Use { session: None, ..a.tool(name) }).contains("no one answered"), "{word}, naming no session");
    }
    // A yes is for the one use: the next asks again (what `first-use` once did not).
    let waiter = a.begin(a.tool("first"), "", LONG);
    let n = a.waiting("alice", 1)[0].n;
    a.m.b.answer_ask(n, "alice", true, false, "terminal").unwrap();
    waiter.join().unwrap().unwrap();
    assert!(a.unanswered(&a.tool("first")).contains("no one answered"), "asked again");
    assert_eq!((a.count("secret_use_approved"), a.count("secret_first_use")), (1, 0), "on the record once, as approved");
    // The standing word.
    assert_eq!(used(&a.m.b, &a.tool("std")).unwrap().value, VALUE, "without asking");
    assert_eq!(used(&a.m.b, &Use { session: None, ..a.tool("std") }).unwrap().value, VALUE);
    assert_eq!(a.count("secret_use_approved"), 1, "no one was asked for it");
    // The same rules in today's words do the same.
    let repo = a.m.b.repo.root().to_path_buf();
    let rules = fs::read_to_string(repo.join("users/alice/secrets.json")).unwrap();
    fs::write(repo.join("users/alice/secrets.json"), rules.replace("never-even-tainted", "without-asking").replace("\"never\"", "\"each-use\"").replace("first-use", "each-use")).unwrap();
    assert!(std::process::Command::new("git").arg("-C").arg(&repo).args(["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qam", "today's words"]).status().unwrap().success());
    assert_eq!(used(&a.m.b, &a.tool("std")).unwrap().value, VALUE);
    assert!(a.unanswered(&a.tool("gh")).contains("no one answered") && a.unanswered(&a.tool("first")).contains("no one answered"));
}

/// Whatever an entry on the record says, the kinds of grant that went let nothing through: a ledger is text,
/// and one from elsewhere (or from before 2026-11-16) is someone else's. All of a person's secrets, whatever
/// else it says; one secret without the word for holding whatever the agent read; one secret for "this session".
#[test]
fn an_entry_of_a_kind_of_grant_that_went_is_not_honoured() {
    let a = Asked::new();
    let until = (chrono::Utc::now() + chrono::Duration::minutes(30)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let entry = |data: serde_json::Value| {
        let mut all = serde_json::json!({ "person": "alice", "minutes": 30, "until": until });
        for (k, v) in data.as_object().unwrap() {
            all[k] = v.clone();
        }
        a.m.b.ledger.append(torq_broker::ledger::NewEntry { actor: "user".into(), approved_by: Some("alice".into()), kind: "trust_granted".into(), tier: "user".into(), tainted: false, reason: "from elsewhere".into(), data: all }).unwrap();
    };
    entry(serde_json::json!({ "covers": "all-secrets", "tainted_too": true }));
    entry(serde_json::json!({ "covers": "all-secrets" }));
    entry(serde_json::json!({ "covers": "secret each", "owner": "user" }));
    entry(serde_json::json!({ "covers": "secret each", "owner": "user", "scope": "session" }));
    entry(serde_json::json!({ "covers": "secret each", "owner": "user", "scope": "session", "tainted_too": true }));
    assert!(session::active(&a.m.b.ledger.read_all().unwrap()).is_empty(), "none of them is a grant");
    assert!(a.unanswered(&a.tool("each")).contains("no one answered"), "so the use asks");
    // And none of them stands in the way of a grant of today's kind, or is ended by a revoke.
    session::grant_secret(&a.m.b, "alice", Covers { name: "each".into(), system: false }, 5, false).unwrap();
    assert_eq!(used(&a.m.b, &a.tool("each")).unwrap().value, VALUE);
    assert_eq!(session::revoke_secrets(&a.m.b, "alice", &Ending::Everything).unwrap().len(), 1, "the one grant there is");
    a.m.b.ledger.verify().unwrap();
}

/// A use that must ask is told so at once, with its number: nothing waits at the broker. Asking again about the
/// number is the use's own to do: a number is no key to another use's answer, and an answer is spent once.
#[test]
fn a_use_that_asks_is_told_its_number_at_once_and_only_it_collects_the_answer() {
    let a = Asked::new();
    let t = std::time::Instant::now();
    let Used::Waiting(n) = a.m.b.begin_use(&a.tool("each"), "").unwrap() else { panic!("it asks") };
    assert!(t.elapsed() < WAIT, "told at once: {:?}", t.elapsed());
    assert_eq!(a.m.b.resume_use(&a.tool("each"), n).unwrap(), Used::Waiting(n), "not answered yet");
    a.m.b.answer_ask(n, "alice", true, false, "terminal").unwrap();
    // Approved, and still not anyone's but its own: another secret, host, session, caller or operation.
    let own = Use { owner: alice(), name: "own".into(), host: "api.github.com".into(), actor: "alice".into(), session: None, ssh: false };
    for other in [a.tool("gh"), a.tool("first"), a.second("each"), Use { host: "evil.example.com".into(), ..a.tool("each") }, Use { ssh: true, ..a.tool("each") }, Use { owner: Owner::System, ..a.tool("each") }, Use { actor: "torq-agent-bob".into(), ..a.tool("each") }, own] {
        let e = format!("{:#}", a.m.b.resume_use(&other, n).unwrap_err());
        assert!(e.contains("is not this use") || e.contains("only an agent's use waits") || e.contains("refused: session"), "{other:?}: {e}");
    }
    let Used::Granted(g) = a.m.b.resume_use(&a.tool("each"), n).unwrap() else { panic!("approved") };
    assert_eq!(g.value, VALUE);
    let again = format!("{:#}", a.m.b.resume_use(&a.tool("each"), n).unwrap_err());
    assert!(again.contains("no one answered"), "an answer is spent once: {again}");
    assert_eq!((a.count("secret_use_approved"), a.count("secret_use_timed_out")), (1, 0));
}

/// An answer given in a use's last moment is an answer: the use that comes back for it just after its time
/// still has it.
#[test]
fn an_answer_in_the_last_moment_is_still_collected() {
    let a = Asked::new();
    let Used::Waiting(n) = a.m.b.begin_use(&a.tool("each"), "").unwrap() else { panic!("it asks") };
    a.m.b.answer_ask(n, "alice", true, false, "terminal").unwrap();
    std::thread::sleep(WAIT + std::time::Duration::from_millis(200));
    assert!(a.m.b.waiting_asks("alice").unwrap().is_empty(), "whatever else looks at the store meanwhile");
    assert!(format!("{:#}", a.m.b.answer_ask(n, "alice", false, false, "terminal").unwrap_err()).contains("answered already"));
    assert!(matches!(a.m.b.resume_use(&a.tool("each"), n).unwrap(), Used::Granted(_)));
    assert_eq!((a.count("secret_use_approved"), a.count("secret_use_timed_out"), a.count("secret_use_denied")), (1, 0, 0));
}

/// A person is asked about one agent's uses at most thirty times in an hour, whatever sessions they name (an agent
/// starts a new session when it likes); uses that name no session, at most ten of those. The next is refused at
/// once, saying so, and on the record as a refusal.
#[test]
fn an_agent_asks_at_most_thirty_times_an_hour_across_its_sessions() {
    let a = Asked::new();
    // Each asked, and denied at once: what is counted is the asking.
    let asked = |u: &Use| -> Result<(), String> {
        match a.m.b.begin_use(u, "").map_err(|e| format!("{e:#}"))? {
            Used::Waiting(n) => a.m.b.answer_ask(n, "alice", false, false, "terminal").map(drop).map_err(|e| format!("{e:#}")),
            Used::Granted(_) => Err("it was not asked".into()),
        }
    };
    let nowhere = Use { session: None, ..a.tool("each") };
    for i in 0..10 {
        asked(&nowhere).unwrap_or_else(|e| panic!("{i}: {e}"));
    }
    let e = asked(&nowhere).unwrap_err();
    assert!(e.contains("asked 10 times about uses that name no session"), "{e}");
    // That one counted too (11 so far): nineteen more are asked, in sessions started as they are wanted.
    let mut session = String::new();
    for i in 0..19 {
        if i % 10 == 0 {
            session = session::start(&a.m.as_agent("torq-agent-alice")).unwrap().id;
        }
        asked(&Use { session: Some(session.clone()), ..a.tool("each") }).unwrap_or_else(|e| panic!("{i}: {e}"));
    }
    assert_eq!(a.count("secret_use_denied"), 29);
    let fresh = session::start(&a.m.as_agent("torq-agent-alice")).unwrap().id;
    let e = asked(&Use { session: Some(fresh.clone()), ..a.tool("each") }).unwrap_err();
    assert!(e.contains("asked 30 times about this agent's uses this hour"), "{e}");
    assert!(a.m.b.waiting_asks("alice").unwrap().is_empty(), "no one is asked");
    assert_eq!(a.entries("secret_use_refused").iter().filter(|e| e["data"]["error"].as_str().is_some_and(|s| s.contains("30 times"))).count(), 1);
    assert_eq!(used(&a.m.b, &Use { session: Some(fresh), ..a.tool("std") }).unwrap().value, VALUE, "what asks no one still goes");
    // Another person's agent has its own count.
    let bobs = session::start(&a.m.as_agent("torq-agent-bob")).unwrap().id;
    let Used::Waiting(n) = a.m.b.begin_use(&Use { owner: Owner::Person("bob".into()), name: "gh".into(), host: "api.github.com".into(), actor: "torq-agent-bob".into(), session: Some(bobs), ssh: false }, "").unwrap() else { panic!("bob is asked") };
    a.m.b.answer_ask(n, "bob", false, false, "terminal").unwrap();
}

/// A denial for the rest of the session that cannot be kept in the session's record is still one outcome: the
/// use is taken away refused, never left to be answered again or to time out on the record as well.
#[test]
fn a_denial_that_cannot_be_kept_still_ends_the_use_once() {
    let a = Asked::new();
    let Used::Waiting(n) = a.m.b.begin_use(&a.tool("each"), "").unwrap() else { panic!("it asks") };
    let sessions = a.m.dir.path().join("state/sessions");
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o500)).unwrap();
    let e = a.m.b.answer_ask(n, "alice", false, true, "terminal");
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).unwrap();
    let e = format!("{:#}", e.unwrap_err());
    assert!(e.contains("the use is refused"), "{e}");
    assert!(a.m.b.answer_ask(n, "alice", true, false, "terminal").is_err(), "not answered again");
    assert!(a.m.b.resume_use(&a.tool("each"), n).is_err(), "and it does not go through");
    assert!(a.m.b.waiting_asks("alice").unwrap().is_empty());
    assert_eq!((a.count("secret_use_denied"), a.count("secret_use_approved"), a.count("secret_use_timed_out")), (1, 0, 0), "one outcome");
}
