//! What the agent is for this person and on this machine (ADR-051 2 and 4), as the Agent page, the applet
//! and the window say it: their agent account, whether its service is here, and what answers — the engine
//! (ADR-055, from /etc/torq/agent-tools.json), or the model TorqOS's own `torq doctor` asks —
//! local, or REMOTE and where — read from what the system's configuration put in /etc/torq/agent.json
//! (modules/ai.nix), everyone's to read. Whether a key is configured is a yes or a no: its file's name is
//! in the configuration, its contents are the agent's, never read here.

use serde::Deserialize;

/// What the system's configuration wrote for the agent (modules/ai.nix): the fields this reads.
const AGENT_JSON: &str = "/etc/torq/agent.json";

#[derive(Deserialize)]
struct Written {
    endpoint: String,
    model: String,
    #[serde(rename = "apiKeyFile", default)]
    api_key_file: Option<String>,
}

/// Where the model is.
#[derive(Debug, Clone, PartialEq)]
pub enum Place {
    /// llama-server on this machine, on its socket (the default).
    Local,
    /// Another server on this machine (`http://127.0.0.1…`): what it reads stays here.
    ThisMachine(String),
    /// Off the machine: its host. What the agent reads leaves the machine.
    Remote(String),
    /// The engine's own (ADR-055): the coding tool asks its maker's service itself, as the agent, with the
    /// sign-in kept in the agent's account. What the agent reads leaves the machine.
    Engine(&'static str),
}

/// The model the agent asks.
#[derive(Debug, Clone, PartialEq)]
pub struct Brain {
    pub place: Place,
    /// The model's name, as its endpoint knows it.
    pub model: String,
    /// The endpoint, as configured.
    pub endpoint: String,
    /// Whether the configuration names a key file (never its contents).
    pub key: bool,
}

/// The host (and port) of an `https://` or `http://` URL.
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // Never a user or password in what is shown.
    crate::history::one_line(host.rsplit('@').next().unwrap_or_default(), 80)
}

/// A URL as it is shown: its scheme, host and path — never a user, password, query or fragment, where a key
/// could be (the configuration refuses those too, modules/ai.nix).
fn shown_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else { return url.split(['?', '#']).next().unwrap_or_default().to_string() };
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let end = rest.find('/').unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    format!("{scheme}://{}{path}", authority.rsplit('@').next().unwrap_or_default())
}

/// /etc/torq/agent.json's words, read.
pub fn brain_of(json: &str) -> anyhow::Result<Brain> {
    let w: Written = serde_json::from_str(json)?;
    let place = if w.endpoint.starts_with("unix:") {
        Place::Local
    } else if w.endpoint.starts_with("https://") {
        Place::Remote(host_of(&w.endpoint))
    } else {
        Place::ThisMachine(host_of(&w.endpoint))
    };
    Ok(Brain { place, model: crate::history::one_line(&w.model, 80), endpoint: crate::history::one_line(&shown_url(&w.endpoint), 200), key: w.api_key_file.is_some() })
}

/// The engine `json` (agent-tools.json) names, if it is one TorqOS knows: what it is called.
pub fn engine_of(json: &str) -> Option<&'static str> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let id = v["engine"].as_str()?;
    KNOWN_TOOLS.iter().find(|(known, _, _, _)| *known == id).map(|(_, name, _, _)| *name)
}

/// What answers this machine's agents: the engine, where the configuration names one (`torq agent engine`);
/// else the model TorqOS's own `torq doctor` asks, where doctor mode is on; else `None`.
pub fn brain() -> anyhow::Result<Option<Brain>> {
    if let Some(name) = engine_of(&std::fs::read_to_string(TOOLS_JSON).unwrap_or_default()) {
        return Ok(Some(Brain { place: Place::Engine(name), model: name.to_string(), endpoint: String::new(), key: false }));
    }
    match std::fs::read_to_string(AGENT_JSON) {
        Ok(json) => brain_of(&json).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::Error::from(e).context(AGENT_JSON)),
    }
}

impl Brain {
    /// In one line: "Local model (llama-server on this machine)", "REMOTE: api.anthropic.com · claude-…".
    pub fn line(&self) -> String {
        match &self.place {
            Place::Local => "Local model (llama-server on this machine)".to_string(),
            Place::ThisMachine(h) => format!("On this machine: {h} · {}", self.model),
            Place::Remote(h) => format!("REMOTE: {h} · {}", self.model),
            // Said by name where TorqOS knows whose it is: what the agent reads leaves the machine, to them.
            Place::Engine(name) => format!("{name} · what it reads goes to {}", KNOWN_TOOLS.iter().find(|(_, known, _, _)| known == name).map_or("its maker's service", |(_, _, _, maker)| *maker)),
        }
    }

    pub fn remote(&self) -> bool {
        matches!(self.place, Place::Remote(_) | Place::Engine(_))
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
    /// The model, where doctor mode is on.
    pub brain: Option<Brain>,
    /// The coding tools the configuration installs for the agents (`torq agent tools`, ADR-051 5).
    pub tools: Vec<CodingTool>,
    /// Whether `claude`, typed plainly, is a command on this machine: what the person is told to type.
    pub plain: bool,
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
    Ok(Setup { me, agent, service, brain: brain()?, tools, plain: plain_command_of(&written) })
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

    #[test]
    fn the_model_in_one_line() {
        let local = brain_of(r#"{"endpoint":"unix:/run/llama-cpp/llama.sock","model":"local","apiKeyFile":null,"journal":false,"maxSteps":8}"#).unwrap();
        assert_eq!(local.place, Place::Local);
        assert_eq!(local.line(), "Local model (llama-server on this machine)");
        assert!(!local.key && !local.remote());
        let remote = brain_of(r#"{"endpoint":"https://key:secret@api.anthropic.com/v1","model":"claude-sonnet-4-5","apiKeyFile":"/var/lib/torq-agent-alice/model.key","journal":true,"maxSteps":8}"#).unwrap();
        assert_eq!(remote.place, Place::Remote("api.anthropic.com".into()));
        assert_eq!(remote.line(), "REMOTE: api.anthropic.com · claude-sonnet-4-5");
        assert!(remote.key && remote.remote());
        assert_eq!(remote.endpoint, "https://api.anthropic.com/v1", "no password shown");
        let keyed = brain_of(r#"{"endpoint":"https://api.example.org/v1?key=secret#more","model":"m"}"#).unwrap();
        assert_eq!(keyed.endpoint, "https://api.example.org/v1", "no query or fragment shown");
        assert_eq!(shown_url("https://h?k=s"), "https://h");
        assert_eq!(shown_url("https://u:p@h/p#f"), "https://h/p");
        let here = brain_of(r#"{"endpoint":"http://127.0.0.1:8099/v1","model":"fake","apiKeyFile":null}"#).unwrap();
        assert_eq!(here.place, Place::ThisMachine("127.0.0.1:8099".into()));
        assert!(!here.remote());
        assert!(brain_of("{}").is_err());
    }

    /// ADR-055: the engine the configuration names, by what it is called; none named, or one this TorqOS does
    /// not know, is no engine. Where there is one, it is what answers, said remote.
    #[test]
    fn the_engine_the_configuration_names() {
        assert_eq!(engine_of(r#"{"tools":["claude-code"],"engine":"claude-code"}"#), Some("Claude Code"));
        for none in [r#"{"tools":["claude-code"],"engine":null}"#, r#"{"tools":["claude-code"]}"#, r#"{"engine":"something-else"}"#, "", "not json"] {
            assert_eq!(engine_of(none), None, "{none}");
        }
        let b = Brain { place: Place::Engine("Claude Code"), model: "Claude Code".into(), endpoint: String::new(), key: false };
        assert_eq!(b.line(), "Claude Code · what it reads goes to Anthropic");
        let other = Brain { place: Place::Engine("Something else"), ..b.clone() };
        assert_eq!(other.line(), "Something else · what it reads goes to its maker's service");
        assert!(b.remote());
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
