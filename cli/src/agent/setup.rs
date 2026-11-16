//! What the agent is for this person and on this machine (ADR-051 2 and 4, ADR-055), as the Agent page, the
//! applet and the window say it: their agent account, whether its service is here, and what answers — the
//! engine, the person's coding tool, from what the system's configuration put in /etc/torq/agent-tools.json
//! (modules/agent.nix), everyone's to read.

/// What answers when a person asks their agent: the engine the configuration names (`torq agent engine`).
#[derive(Debug, Clone, PartialEq)]
pub struct Engine {
    /// What it is called.
    pub name: &'static str,
    /// Whose service it asks: where what the agent reads goes.
    pub maker: &'static str,
}

/// The engine `json` (agent-tools.json) names, if it is one TorqOS knows.
pub fn engine_of(json: &str) -> Option<Engine> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let id = v["engine"].as_str()?;
    KNOWN_TOOLS.iter().find(|(known, _, _, _)| *known == id).map(|(_, name, _, maker)| Engine { name, maker })
}

/// What answers this machine's agents: the engine the configuration names; none named, `None`.
pub fn engine() -> Option<Engine> {
    engine_of(&std::fs::read_to_string(TOOLS_JSON).unwrap_or_default())
}

impl Engine {
    /// In one line, with where what the agent reads goes: "Claude Code · what it reads goes to Anthropic".
    pub fn line(&self) -> String {
        format!("{} · what it reads goes to {}", self.name, self.maker)
    }
}

/// The person and their agent, for the Agent page.
#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    pub me: String,
    /// Their agent account, if they have one.
    pub agent: Option<String>,
    /// Whether their agent's service is on this machine (its socket is there).
    pub service: bool,
    /// What answers, where an engine is chosen.
    pub engine: Option<Engine>,
    /// The coding tools the configuration installs for the agents (`torq agent tools`, ADR-051 5).
    pub tools: Vec<CodingTool>,
    /// Whether `claude`, typed plainly, is a command on this machine: what the person is told to type.
    pub plain: bool,
    /// The person's standing instructions for their agent, as they stood when this was read.
    pub instructions: super::instructions::Instructions,
}

/// A coding tool the agents may run (ADR-051 5): what it is, and whether this person's agent has it.
#[derive(Debug, Clone, PartialEq)]
pub struct CodingTool {
    /// Its name in the tree (`torq agent tools`): `claude-code`.
    pub id: String,
    /// What it is called.
    pub name: &'static str,
    /// Whether the person's agent has its program (in the agent's own profile).
    pub installed: bool,
    /// How the person starts it, in a terminal: its own name where that is a command here, in the folder they
    /// want it to work in; else spelled out, as their agent, in a folder they share.
    pub start: String,
}

/// What the system's configuration wrote of the coding tools (modules/agent.nix).
const TOOLS_JSON: &str = "/etc/torq/agent-tools.json";

/// The coding tools TorqOS knows (only Claude Code in this step), each with what it is called, its program's name,
/// and whose service it asks (where what it reads goes).
const KNOWN_TOOLS: &[(&str, &str, &str, &str)] = &[("claude-code", "Claude Code", "claude", "Anthropic")];

/// Whether `claude`, typed plainly, is a command on this machine (`torq agent plainCommand`, where Claude Code
/// is installed for the agents), as `json` (agent-tools.json) says. Not said: it is not.
pub fn plain_command_of(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json).is_ok_and(|v| v["plainCommand"] == true)
}

/// The coding tools `json` (agent-tools.json) names that TorqOS knows, for `agent`'s account: installed if its
/// program is in that account's own profile (`profile`, its `bin`), as `has` says. How to start each: its own
/// name where that is a command here (`plain_command_of`), else through `torq agent run`.
pub fn tools_of(json: &str, agent: Option<&str>, has: impl Fn(&std::path::Path) -> bool) -> Vec<CodingTool> {
    let named: Vec<String> = serde_json::from_str::<serde_json::Value>(json).ok().and_then(|v| v["tools"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect())).unwrap_or_default();
    let plain = plain_command_of(json);
    KNOWN_TOOLS
        .iter()
        .map(|(id, name, program, _)| CodingTool {
            id: id.to_string(),
            name,
            installed: named.iter().any(|n| n == id) && agent.is_some_and(|a| has(&std::path::Path::new("/etc/profiles/per-user").join(a).join("bin").join(program))),
            start: if plain { program.to_string() } else { format!("torq agent run --share <a folder of yours> -- {program}") },
        })
        .collect()
}

/// The caller's own setup.
pub fn setup() -> anyhow::Result<Setup> {
    let me = crate::me()?;
    let passwd = std::fs::read_to_string("/etc/passwd")?;
    let account = super::agent_of(&me);
    let agent = passwd.lines().any(|l| l.split(':').next() == Some(account.as_str())).then_some(account);
    let service = std::path::Path::new(&torq_wire::agent::socket(&me)).exists();
    let written = std::fs::read_to_string(TOOLS_JSON).unwrap_or_default();
    let tools = tools_of(&written, agent.as_deref(), |p| p.exists());
    Ok(Setup { me, agent, service, engine: engine(), tools, plain: plain_command_of(&written), instructions: super::instructions::mine() })
}

/// How long systemd has to say whether the service runs: a look is never held longer.
const SYSTEMD_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Whether `me`'s agent service runs now — asked of systemd (`systemctl`, by absolute path), not of the
/// service, so that looking never starts it (its socket starts it when the person first asks).
pub fn service_running(me: &str) -> bool {
    let mut c = std::process::Command::new(crate::app::system_program("systemctl"));
    c.args(["show", "-P", "ActiveState", &format!("torq-agent@{me}.service")]);
    output_within(c, SYSTEMD_WAIT).is_some_and(|o| o.trim() == "active")
}

/// What `c` prints, if it ends well within `limit`; killed, and nothing, if not.
pub(crate) fn output_within(mut c: std::process::Command, limit: std::time::Duration) -> Option<String> {
    use std::io::Read;
    const KEPT: usize = 64 * 1024;
    let mut child = c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
    // Read while it runs: output past a pipe's size would block it until the
    // limit. The first KEPT bytes kept, the rest read and dropped.
    let mut stdout = child.stdout.take()?;
    let kept = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let (ended, read_all) = std::sync::mpsc::channel::<()>();
    let into = kept.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 {
                break;
            }
            let mut k = into.lock().unwrap_or_else(|p| p.into_inner());
            let room = KEPT.saturating_sub(k.len());
            k.extend_from_slice(&buf[..n.min(room)]);
        }
        // Its receiver may have stopped waiting: nothing is left to tell.
        let _ = ended.send(());
    });
    let deadline = std::time::Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    // Its output ends with it, unless something it left holds the pipe open: then, after a moment, what was read
    // by then is its output (a timeout here says only that).
    let _ = read_all.recv_timeout(std::time::Duration::from_secs(2));
    let out = kept.lock().unwrap_or_else(|p| p.into_inner()).clone();
    status.success().then(|| String::from_utf8_lossy(&out).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR-055: the engine the configuration names, by what it is called and whose service it asks; none named,
    /// or one this TorqOS does not know, is no engine.
    #[test]
    fn the_engine_the_configuration_names() {
        let e = engine_of(r#"{"tools":["claude-code"],"engine":"claude-code"}"#).unwrap();
        assert_eq!(e, Engine { name: "Claude Code", maker: "Anthropic" });
        assert_eq!(e.line(), "Claude Code · what it reads goes to Anthropic");
        for none in [r#"{"tools":["claude-code"],"engine":null}"#, r#"{"tools":["claude-code"]}"#, r#"{"engine":"something-else"}"#, "", "not json"] {
            assert_eq!(engine_of(none), None, "{none}");
        }
    }

    /// ADR-051 5: the coding tools the configuration chose, installed where the agent's own profile has them;
    /// how to start each, as the agent, in a shared folder.
    #[test]
    fn the_coding_tools_as_the_agent_page_says_them() {
        let has = |p: &std::path::Path| p == std::path::Path::new("/etc/profiles/per-user/torq-agent-alice/bin/claude");
        let t = tools_of(r#"{"tools":["claude-code"]}"#, Some("torq-agent-alice"), has);
        assert_eq!(t.len(), 1);
        assert!(t[0].installed && t[0].id == "claude-code" && t[0].name == "Claude Code", "{t:?}");
        assert_eq!(t[0].start, "torq agent run --share <a folder of yours> -- claude", "where `claude` is not a command here");
        let plain = tools_of(r#"{"tools":["claude-code"],"plainCommand":true}"#, Some("torq-agent-alice"), has);
        assert_eq!(plain[0].start, "claude", "where it is: its own name");
        assert!(plain_command_of(r#"{"plainCommand":true}"#) && !plain_command_of(r#"{"plainCommand":false}"#) && !plain_command_of("{}") && !plain_command_of(""));
        // Not chosen, no agent, another person's agent, nothing written: not installed for this person.
        for (json, agent) in [(r#"{"tools":[]}"#, Some("torq-agent-alice")), (r#"{"tools":["claude-code"]}"#, None), (r#"{"tools":["claude-code"]}"#, Some("torq-agent-bob")), ("", Some("torq-agent-alice")), ("not json", Some("torq-agent-alice"))] {
            assert!(!tools_of(json, agent, has)[0].installed, "{json} {agent:?}");
        }
    }

    /// A systemd that does not answer never holds a look: killed at the limit.
    #[test]
    fn systemd_asked_within_a_limit() {
        let mut quick = std::process::Command::new("sh");
        quick.args(["-c", "echo active"]);
        assert_eq!(output_within(quick, std::time::Duration::from_secs(5)).as_deref(), Some("active\n"));
        let mut slow = std::process::Command::new("sh");
        slow.args(["-c", "sleep 30; echo active"]);
        let t = std::time::Instant::now();
        assert_eq!(output_within(slow, std::time::Duration::from_millis(300)), None);
        assert!(t.elapsed() < std::time::Duration::from_secs(5), "{:?}", t.elapsed());
        let mut failed = std::process::Command::new("sh");
        failed.args(["-c", "echo active; exit 1"]);
        assert_eq!(output_within(failed, std::time::Duration::from_secs(5)), None);
    }

    /// Output past what a pipe holds is read while the program runs: a crash
    /// summary over 64 KiB was never read, the program blocked writing it until the time limit. Kept up to 64 KiB,
    /// the rest read and dropped; not UTF-8, read as it can be.
    #[test]
    fn a_long_output_is_read_while_it_runs() {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", "head -c 300000 /dev/zero | tr '\\0' a; printf '\\377'"]);
        let t = std::time::Instant::now();
        let out = output_within(c, std::time::Duration::from_secs(10)).expect("read, not timed out");
        assert!(t.elapsed() < std::time::Duration::from_secs(5), "{:?}", t.elapsed());
        assert_eq!(out.len(), 64 * 1024);
        assert!(out.bytes().all(|b| b == b'a'));
        let mut c = std::process::Command::new("sh");
        c.args(["-c", "printf 'ok \\377'"]);
        assert_eq!(output_within(c, std::time::Duration::from_secs(10)).as_deref(), Some("ok \u{fffd}"));
        // Something it leaves holding the pipe does not keep it waiting past a moment.
        let mut c = std::process::Command::new("sh");
        c.args(["-c", "echo left; sleep 30 &"]);
        let t = std::time::Instant::now();
        assert_eq!(output_within(c, std::time::Duration::from_secs(10)).as_deref(), Some("left\n"));
        assert!(t.elapsed() < std::time::Duration::from_secs(5), "{:?}", t.elapsed());
    }
}
