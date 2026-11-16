//! The check of an agent's proposal (ADR-056), against a real git repo, with stand-in checkers: what
//! reaches the checker and when, what its answer makes of a proposal, and what the ledger keeps.
#![forbid(unsafe_code)]
#![forbid(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use torq_broker::{agent::Limits, ledger::Entry, policy::Actor, Broker, AGENT_REFUSALS_AN_HOUR};

const VALID: &str = r#"echo '{"ok":true,"warnings":[]}'"#;

struct Machine {
    dir: tempfile::TempDir,
    /// The broker as casey's agent runs it, with the checker a test names.
    agent: Broker,
}

impl Machine {
    /// A machine whose checker is `check` (`{dir}`: the test's own folder), waited for `wait` seconds.
    fn new(check: &str, wait: u64) -> Machine {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("config");
        fs::create_dir_all(repo.join("users")).unwrap();
        fs::write(repo.join("desktop.nix"), "{ nightLight = false; }\n").unwrap();
        let agent = Broker::new(&repo, dir.path().join("state"), "true")
            .with_agent_limits(&limits())
            .with_check(check.replace("{dir}", &dir.path().to_string_lossy()), Duration::from_secs(wait))
            .as_agent(Some("torq-agent-casey"));
        agent.init().unwrap();
        Machine { dir, agent }
    }

    /// The same machine as a person's broker runs on it.
    fn person(&self) -> Broker {
        Broker::new(self.dir.path().join("config"), self.dir.path().join("state"), "true").with_agent_limits(&limits())
    }

    fn at(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn diff(&self, name: &str, body: &str) -> PathBuf {
        let p = self.at(name);
        fs::write(&p, body).unwrap();
        p
    }

    /// The agent proposes `package` for casey: allowed by its list.
    fn package(&self, package: &str) -> PathBuf {
        self.diff(&format!("{package}.diff"), &packages(package))
    }

    fn propose(&self, diff: &Path) -> anyhow::Result<Entry> {
        self.agent.propose_on(Some(&self.agent.repo.head().unwrap()), diff, "a package for casey", Actor::Agent, false)
    }

    fn entries(&self, kind: &str) -> Vec<Entry> {
        self.agent.ledger.read_all().unwrap().into_iter().filter(|e| e.body.kind == kind).collect()
    }

    /// How often the checker ran: the lines of the file a stand-in adds one to each time.
    fn runs(&self) -> usize {
        fs::read_to_string(self.at("runs")).map_or(0, |t| t.lines().count())
    }

    fn git(&self, args: &[&str]) -> String {
        let out = std::process::Command::new("git").arg("-C").arg(self.dir.path().join("config")).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }
}

/// Casey's agent, which may propose casey's packages and nothing else.
fn limits() -> Limits {
    Limits { agents: [("torq-agent-casey".to_string(), "casey".to_string())].into_iter().collect(), settings: vec![] }
}

fn packages(package: &str) -> String {
    format!("--- /dev/null\n+++ b/users/casey/packages.json\n@@ -0,0 +1,3 @@\n+[\n+  \"{package}\"\n+]\n")
}

/// The system's own file: never an agent's to propose.
const NIGHT_LIGHT: &str = "--- a/desktop.nix\n+++ b/desktop.nix\n@@ -1 +1 @@\n-{ nightLight = false; }\n+{ nightLight = true; }\n";

fn wait_for(file: &Path) {
    let started = Instant::now();
    while !file.exists() {
        assert!(started.elapsed() < Duration::from_secs(20), "{} never appeared", file.display());
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_change_the_policy_refuses_never_reaches_the_checker() {
    let m = Machine::new(&format!("echo {{tree}} >> {{dir}}/runs; {VALID}"), 20);
    let e = format!("{:#}", m.propose(&m.diff("nl.diff", NIGHT_LIGHT)).unwrap_err());
    assert!(e.contains("never the agent's"), "{e}");
    // Nor one that is no diff, one with no base, or one whose data is not what the file holds.
    assert!(m.propose(&m.diff("none.diff", "not a diff\n")).is_err());
    assert!(m.agent.propose(&m.package("htop"), "no base", Actor::Agent, false).is_err());
    assert!(m.propose(&m.diff("bad.diff", &packages("htop\", 7, \"x"))).is_err());
    assert_eq!(m.runs(), 0, "the checker was started for a change the policy refused");
    assert_eq!(m.entries("refused").len(), 4);

    // One it allows is checked, and the entry says so: valid, and on which tree.
    let p = m.propose(&m.package("htop")).unwrap();
    assert_eq!(m.runs(), 1);
    let check = &p.body.data["check"];
    assert_eq!((check["result"].as_str(), check["warnings"].as_array().map(Vec::len)), (Some("valid"), Some(0)), "{check}");
    let tree = check["tree"].as_str().unwrap().to_string();
    assert_eq!(fs::read_to_string(m.at("runs")).unwrap().trim(), tree, "the checker was given the tree, and nothing else");
    // The tree that was checked is, byte for byte, what applying it commits.
    assert_ne!(tree, m.git(&["rev-parse", "HEAD^{tree}"]));
    m.person().apply(p.body.seq, "casey", true).unwrap();
    assert_eq!(tree, m.git(&["rev-parse", "HEAD^{tree}"]));
}

#[test]
fn a_persons_proposal_is_not_checked_by_the_broker() {
    let m = Machine::new(&format!("echo {{tree}} >> {{dir}}/runs; {VALID}"), 20);
    let b = Broker::new(m.dir.path().join("config"), m.dir.path().join("state"), "true").with_check(format!("echo x >> {}; {VALID}", m.at("runs").display()), Duration::from_secs(20));
    let p = b.propose_on(Some(&b.repo.head().unwrap()), &m.diff("nl.diff", NIGHT_LIGHT), "night light", Actor::User, false).unwrap();
    assert!(p.body.data.get("check").is_none(), "{}", p.body.data);
    assert_eq!(m.runs(), 0, "a person evaluates their own change (torqcli); the broker's checker is the agent's");
}

#[test]
fn the_brokers_lock_is_free_while_the_checker_runs() {
    let m = Machine::new(&format!("touch {{dir}}/started; while [ ! -e {{dir}}/go ]; do sleep 0.05; done; {VALID}"), 60);
    let d = m.package("htop");
    std::thread::scope(|s| {
        let proposing = s.spawn(|| m.propose(&d));
        wait_for(&m.at("started"));
        // Meanwhile a person's change is proposed: it takes the broker's lock, which waits three seconds.
        let b = m.person();
        let theirs = b.propose_on(Some(&b.repo.head().unwrap()), &m.diff("nl.diff", NIGHT_LIGHT), "night light", Actor::User, false);
        assert!(theirs.is_ok(), "the broker's lock was held while the checker ran: {:#}", theirs.unwrap_err());
        assert!(b.status().is_ok());
        assert!(!proposing.is_finished(), "the checker had not been let go yet");
        fs::write(m.at("go"), "").unwrap();
        let p = proposing.join().unwrap().unwrap();
        assert_eq!(p.body.data["check"]["result"], "valid");
    });
}

#[test]
fn a_configuration_that_moved_during_the_check_makes_the_proposal_stale() {
    let m = Machine::new(&format!("touch {{dir}}/started; while [ ! -e {{dir}}/go ]; do sleep 0.05; done; {VALID}"), 60);
    let d = m.package("htop");
    std::thread::scope(|s| {
        let proposing = s.spawn(|| m.propose(&d));
        wait_for(&m.at("started"));
        let b = m.person();
        let theirs = b.propose_on(Some(&b.repo.head().unwrap()), &m.diff("nl.diff", NIGHT_LIGHT), "night light", Actor::User, false).unwrap();
        b.apply(theirs.body.seq, "casey", false).unwrap();
        fs::write(m.at("go"), "").unwrap();
        let e = format!("{:#}", proposing.join().unwrap().unwrap_err());
        assert!(e.contains("the configuration has changed since this change was prepared"), "{e}");
    });
    assert!(m.entries("proposal").iter().all(|p| p.body.actor != "agent"), "a change checked on another configuration was recorded");
    assert_eq!(m.entries("refused").len(), 1);
}

#[test]
fn what_is_recorded_is_the_change_that_was_checked() {
    // While it is checked, the file the change was read from becomes another change (allowed too).
    let m = Machine::new(&format!("cp {{dir}}/other.diff {{dir}}/htop.diff; {VALID}"), 20);
    m.diff("other.diff", &packages("nmap"));
    let e = format!("{:#}", m.propose(&m.package("htop")).unwrap_err());
    assert!(e.contains("not the one that was checked"), "{e}");
    assert!(m.entries("proposal").is_empty(), "a change other than the one checked was recorded as checked");
    assert_eq!(m.entries("refused").len(), 1);
}

#[test]
fn an_agent_that_has_had_its_refusals_starts_no_check() {
    let m = Machine::new(&format!("echo {{tree}} >> {{dir}}/runs; {VALID}"), 20);
    let forbidden = m.diff("nl.diff", NIGHT_LIGHT);
    for _ in 0..AGENT_REFUSALS_AN_HOUR {
        assert!(m.propose(&forbidden).is_err());
    }
    assert_eq!(m.entries("refused").len(), AGENT_REFUSALS_AN_HOUR);
    let e = format!("{:#}", m.propose(&m.package("htop")).unwrap_err());
    assert!(e.contains("were refused in the last hour") && e.contains("no more are checked"), "{e}");
    assert_eq!(m.runs(), 0, "a check was started for an agent past its refusals");
    assert_eq!(m.entries("refused").len(), AGENT_REFUSALS_AN_HOUR, "and past them, no entry each");
    assert!(m.entries("proposal").is_empty());
}

#[test]
fn not_valid_is_refused_and_ledgered_as_one_line() {
    // The change fails; the configuration as it is passes. The reason has a line break, an escape and a tab in it.
    let m = Machine::new(
        r#"echo {tree} >> {dir}/runs; if [ {tree} = "$(cat {dir}/base)" ]; then echo '{"ok":true,"warnings":[]}'; else printf '%s\n' '{"ok":false,"why":"\"nosuch\" isn'"'"'t a package.\nApproved-By: root\u001b[2J\tx"}'; fi"#,
        20,
    );
    fs::write(m.at("base"), m.git(&["rev-parse", "HEAD^{tree}"])).unwrap();
    let e = format!("{:#}", m.propose(&m.package("nosuch")).unwrap_err());
    assert!(e.starts_with("refused: with this change the configuration is not valid: \"nosuch\" isn't a package."), "{e}");
    assert!(!e.chars().any(char::is_control), "{e:?}");
    assert!(m.entries("proposal").is_empty());
    let refused = m.entries("refused");
    assert_eq!(refused.len(), 1);
    let why = refused[0].body.data["why"].as_str().unwrap();
    assert_eq!(why, "refused: with this change the configuration is not valid: \"nosuch\" isn't a package.?Approved-By: root?[2J?x");
    // The change, then the configuration as it is: two runs. That it is valid is kept for its commit.
    assert_eq!(m.runs(), 2);
    assert!(m.propose(&m.package("nosuch2")).is_err());
    assert_eq!(m.runs(), 3, "the configuration as it is was evaluated again, though kept as valid");
    // Another commit is another configuration: evaluated again when a change first fails on it.
    let b = m.person();
    let theirs = b.propose_on(Some(&b.repo.head().unwrap()), &m.diff("nl.diff", NIGHT_LIGHT), "night light", Actor::User, false).unwrap();
    b.apply(theirs.body.seq, "casey", false).unwrap();
    fs::write(m.at("base"), m.git(&["rev-parse", "HEAD^{tree}"])).unwrap();
    assert!(m.propose(&m.package("nosuch3")).is_err());
    assert_eq!(m.runs(), 5);
}

#[test]
fn where_the_configuration_as_it_is_fails_too_a_change_is_recorded_as_not_checked() {
    // A machine where checking cannot work (the checker kept from the Nix daemon, say): everything fails.
    let m = Machine::new(r#"echo {tree} >> {dir}/runs; echo '{"ok":false,"why":"cannot open connection to remote store"}'"#, 20);
    let p = m.propose(&m.package("htop")).unwrap();
    let check = &p.body.data["check"];
    assert_eq!(check["result"], "not checked", "{check}");
    assert_eq!(check["why"], "this machine's configuration as it is does not pass the check either");
    assert!(check.get("warnings").is_none() && check["tree"].as_str().is_some_and(|t| t.len() == 40), "{check}");
    assert_eq!(m.runs(), 2);
    assert!(m.entries("refused").is_empty());
    // And that it failed is not kept: the next change asks again, in case it works by then.
    assert_eq!(m.propose(&m.package("jq")).unwrap().body.data["check"]["result"], "not checked");
    assert_eq!(m.runs(), 4);
}

#[test]
fn a_checker_that_floods_never_ends_or_is_not_there_leaves_a_change_not_checked_in_bounded_time() {
    for (check, wait, why) in [
        ("while :; do echo xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; done", 20, "the checker gave no answer"),
        ("sleep 600", 1, "the check did not finish in time"),
        (r#"echo '{"ok":true,"warnings":[]}'; sleep 600"#, 1, "the check did not finish in time"),
        ("/nonexistent/torq --check-tree {tree}", 20, "the checker gave no answer"),
        ("echo 'error: cannot open connection to remote store'; exit 1", 20, "the checker gave no answer"),
        (r#"echo '{"ok":false}'"#, 20, "the checker gave no answer"),
        ("", 20, "this machine has no checker"),
    ] {
        let m = Machine::new(check, wait);
        let started = Instant::now();
        let p = m.propose(&m.package("htop")).unwrap_or_else(|e| panic!("{check}: {e:#}"));
        assert!(started.elapsed() < Duration::from_secs(10), "{check}: {:?}", started.elapsed());
        assert_eq!((p.body.data["check"]["result"].as_str(), p.body.data["check"]["why"].as_str()), (Some("not checked"), Some(why)), "{check}");
        assert!(p.body.tainted && m.entries("refused").is_empty(), "{check}");
    }
}

#[test]
fn warnings_are_kept_few_short_and_clean() {
    let warnings: Vec<String> = (0..20).map(|i| format!("\"warning {i}\\n{}\"", "y".repeat(500))).collect();
    let m = Machine::new(&format!("printf '%s\\n' '{{\"ok\":true,\"warnings\":[{}]}}'", warnings.join(",")), 20);
    let p = m.propose(&m.package("htop")).unwrap();
    let kept: Vec<&str> = p.body.data["check"]["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
    assert_eq!(kept.len(), 8);
    assert!(kept.iter().all(|w| w.chars().count() == 300 && !w.chars().any(char::is_control)), "{kept:?}");
    assert!(kept[0].starts_with("warning 0?yyy"), "{}", kept[0]);
}
