//! `torq agent run [--share <dir>] [--] [<command> …]` (ADR-041, 5): a program — Claude Code, or any
//! other — run as the person's own agent account (`torq-agent-<person>`, ADR-051 3), not as the person.
//!
//! It runs as the caller's agent (through `sudo torq-agent-run`, which binds the shared directory as root and
//! then drops to `torq-agent-$SUDO_USER` for good — never an agent the caller names), in
//! the launcher's sandbox (modules/agent-run.sh): every home hidden but the one directory shared with
//! it, the people's runtime directories hidden, /tmp its own. Its home is the agent's, so a tool's
//! settings and credentials are never the person's. What it changes in the system, it proposes —
//! torqcli as the agent stops at the proposal — and a person applies (`request system apply`).
//!
//! `torq agent claude [<words> …]` is what `claude`, typed plainly, runs: Claude Code as the agent, the folder
//! the person is in shared with it (nothing, where that folder is never shared), their standing instructions
//! given to it. `torq agent instructions [edit | clear]` shows and changes those (`instructions`). `torq ask` is
//! the terminal's way to the agent's service (`client`).
//!
//! Sharing a directory is the person's act, and visible: an ACL on what of it is theirs (and default
//! ACLs, so that files either of them makes stay both of theirs). The agent gets no way into their
//! home: torqcli opens the directory, as the person, and the launcher binds that open descriptor —
//! everything around it stays out of the agent's reach, by name or otherwise.

/// The person's own agent, as every program of theirs reaches it (ADR-051 4): its client, what a
/// conversation shows, and what the agent is here.
pub mod client;
pub mod conversation;
pub mod crash;
pub mod handoff;
pub mod instructions;
pub mod launcher;
pub mod setup;
/// What the agent says, as its service streams it (torq-wire's, for the programs built on this library).
pub use torq_wire::agent::Event;

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// What every agent account's name begins with: each person's is `torq-agent-<person>` (ADR-051 3).
const AGENT_PREFIX: &str = "torq-agent-";

/// The agent account of `person`: `torq-agent-<person>`.
pub(crate) fn agent_of(person: &str) -> String {
    format!("{AGENT_PREFIX}{person}")
}

/// What each person's resident agent's own account's name begins with: `torq-resident-<person>` (ADR-051 3,
/// amended 2026-11-11). It runs the person's agent service; to torqcli it is an agent as the agent account is.
const RESIDENT_PREFIX: &str = "torq-resident-";

/// Whether `name` is named as an agent account is (`torq-agent-<someone>`, or a resident agent's own
/// `torq-resident-<someone>`). Convenience only: torqcli
/// runs unprivileged, as whoever runs it, and what it decides here is how it behaves (a commit that
/// proposes and stops, no `torq agent run` from inside one) — never what is allowed. The broker is the
/// authority: it knows an agent by its map (/etc/torq/broker.json) and the kernel's uid, not by a name.
pub(crate) fn is_agent(name: &str) -> bool {
    [AGENT_PREFIX, RESIDENT_PREFIX].iter().any(|pre| name.strip_prefix(pre).is_some_and(|p| !p.is_empty()))
}

const SUDO: &str = "/run/wrappers/bin/sudo";
const LAUNCHER: &str = "/run/current-system/sw/bin/torq-agent-run";
const SETFACL: &str = "/run/current-system/sw/bin/setfacl";
const GETFACL: &str = "/run/current-system/sw/bin/getfacl";

/// What `torq agent run` was asked: the directory to share, and the program (none: a shell).
#[derive(Debug, PartialEq)]
pub(crate) struct Run {
    pub share: Option<PathBuf>,
    pub command: Vec<String>,
}

/// `run [--share <dir>] [--] [<command> …]`, after `agent`.
pub(crate) fn parse(args: &[String]) -> Result<Run> {
    let usage = "usage: torq agent run [--share <folder>] [--] [<command> …] — a program as your agent. Also: torq agent claude [<words for Claude Code> …] — Claude Code as your agent, in the folder you are in; torq agent instructions [edit | clear] — what your agent is always told";
    let mut it = args.iter();
    if it.next().map(String::as_str) != Some("run") {
        bail!("{usage}");
    }
    let rest: Vec<&String> = it.collect();
    let (share, rest) = match rest.as_slice() {
        [flag, dir, rest @ ..] if flag.as_str() == "--share" => (Some(PathBuf::from(dir.as_str())), rest),
        [flag] if flag.as_str() == "--share" => bail!("--share needs a directory\n{usage}"),
        rest => (None, rest),
    };
    let (dashes, rest) = match rest {
        [d, rest @ ..] if d.as_str() == "--" => (true, rest),
        rest => (false, rest),
    };
    if !dashes && rest.first().is_some_and(|a| a.starts_with('-')) {
        bail!("unknown option {}\n{usage}", rest[0]);
    }
    Ok(Run { share, command: rest.iter().map(|s| s.to_string()).collect() })
}

/// Why a folder is not one to share with the agent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum NotShared {
    /// It is not there (or cannot be read).
    Gone,
    NotADirectory,
    /// The person's home itself.
    Home,
    /// Not inside the person's home.
    Outside,
    /// A hidden folder directly under the home (`~/.ssh`, `~/.config`), or inside one: keys and settings live there.
    Hidden,
    /// It belongs to another account.
    NotYours,
}

impl NotShared {
    /// Why, as `claude` typed plainly says it of the folder the person is in.
    fn here(self) -> &'static str {
        match self {
            NotShared::Gone => "this folder is not there any more",
            NotShared::NotADirectory => "this is not a folder",
            NotShared::Home => "this is your home folder itself",
            NotShared::Outside => "this folder is outside your home",
            NotShared::Hidden => "this is a hidden folder of your home, where keys and settings live",
            NotShared::NotYours => "this folder belongs to another account",
        }
    }
}

/// The directory to share, checked: it exists, it is a directory, it is the person's own, and it is
/// inside their home but not the home itself, nor a hidden folder directly under the home (or anything
/// inside one) — named as it is, every link resolved.
pub(crate) fn shareable(dir: &Path, home: &Path, uid: u32) -> Result<PathBuf, NotShared> {
    use std::os::unix::fs::MetadataExt;
    let dir = std::fs::canonicalize(dir).map_err(|_| NotShared::Gone)?;
    let home = std::fs::canonicalize(home).map_err(|_| NotShared::Outside)?;
    let m = std::fs::metadata(&dir).map_err(|_| NotShared::Gone)?;
    if !m.is_dir() {
        return Err(NotShared::NotADirectory);
    }
    if dir == home {
        return Err(NotShared::Home);
    }
    let Ok(inside) = dir.strip_prefix(&home) else { return Err(NotShared::Outside) };
    // ~/.ssh, ~/.config, ~/.gnupg, ~/.local …: the person's keys, sign-ins and settings. Only the first name
    // under the home is looked at: a hidden folder inside a project (its .git) is the project's.
    if inside.components().next().is_some_and(|c| c.as_os_str().as_encoded_bytes().starts_with(b".")) {
        return Err(NotShared::Hidden);
    }
    if m.uid() != uid {
        return Err(NotShared::NotYours);
    }
    Ok(dir)
}

/// The same, for `torq agent run --share <dir>`: each refusal says why, in words for the person who named it.
pub(crate) fn checked_share(dir: &Path, home: &Path, uid: u32) -> Result<PathBuf> {
    let shown = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let shown = shown.display();
    shareable(dir, home, uid).map_err(|why| match why {
        NotShared::Gone => anyhow::anyhow!("{}: no such folder", dir.display()),
        NotShared::NotADirectory => anyhow::anyhow!("{shown}: not a folder"),
        NotShared::Home => anyhow::anyhow!("{shown} is your home folder itself, and your agent is never given all of it: share a folder inside it (a project's, for one)"),
        NotShared::Outside => anyhow::anyhow!("{shown} is not inside your home ({}): only a folder inside your home can be shared with your agent", home.display()),
        NotShared::Hidden => anyhow::anyhow!("{shown} is a hidden folder of your home, or inside one (its name begins with a dot, like ~/.ssh or ~/.config). Your keys, sign-ins and settings live there, so it is never shared with your agent: share a folder of your own work instead"),
        NotShared::NotYours => anyhow::anyhow!("{shown}: not yours to share (it belongs to another account)"),
    })
}

/// Everything under `dir` (itself included) that `uid` owns — its directories, and its other files —
/// walked without following a link (a link is not shared; what it points to is not walked).
pub(crate) fn owned(dir: &Path, uid: u32) -> Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    use std::os::unix::fs::MetadataExt;
    let (mut dirs, mut files, mut todo) = (Vec::new(), Vec::new(), vec![dir.to_path_buf()]);
    while let Some(p) = todo.pop() {
        let m = std::fs::symlink_metadata(&p).with_context(|| format!("reading {}", p.display()))?;
        if m.file_type().is_symlink() || m.uid() != uid {
            continue;
        }
        if m.is_dir() {
            for e in std::fs::read_dir(&p).with_context(|| format!("reading {}", p.display()))? {
                todo.push(e?.path());
            }
            dirs.push(p);
        } else if m.is_file() {
            files.push(p);
        }
    }
    Ok((dirs, files))
}


/// Whether this machine has agent account `agent`, from /etc/passwd: refused, plainly, if not (the
/// person is not in `torq.agent.people`). The launcher checks it again, as root, against the broker's map.
pub(crate) fn has_agent(agent: &str) -> Result<()> {
    let passwd = std::fs::read_to_string("/etc/passwd").context("reading /etc/passwd")?;
    if !passwd.lines().any(|l| l.split(':').next() == Some(agent)) {
        bail!("you have no agent account on this machine ({agent}): `torq agent people` names who has one (the owner, by default) — adding you is a system change, an administrator's");
    }
    Ok(())
}

/// The person's agent held still (ADR-051 3, step 2): the launcher, as root, freezes their agent's
/// service (`systemctl freeze torq-agent@<person>.service`, systemd's freezer: every process of it
/// stopped where it is), refuses if any other process of the agent's runs (another `torq agent run`),
/// says `frozen`, and thaws when its input ends — when this closes it, or exits, however. A program
/// of the agent's running during the walk could swap a file it can write for a link, between the
/// walk's look and its ACL, and be granted wherever the link leads (the review of ADR-041, 5).
struct Frozen(std::process::Child);

impl Frozen {
    fn hold(launcher: &str) -> Result<Frozen> {
        use std::io::BufRead;
        use std::process::Stdio;
        let mut child = Command::new(SUDO).args(["--", launcher, "--hold-frozen"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().context("holding your agent still (sudo torq-agent-run)")?;
        let mut said = String::new();
        let read = std::io::BufReader::new(child.stdout.take().context("the launcher's output")?).read_line(&mut said);
        if read.is_err() || said.trim() != "frozen" {
            drop(child.stdin.take());
            let _ = child.wait();
            bail!("nothing was shared and nothing was started (the line above says why)");
        }
        Ok(Frozen(child))
    }

    /// Thawed: the launcher's input ended, and it says whether the thaw went through.
    fn thaw(mut self) -> Result<()> {
        drop(self.0.stdin.take());
        if !self.0.wait().context("thawing your agent")?.success() {
            bail!("your agent could not be thawed: systemctl thaw it (the launcher said why)");
        }
        Ok(())
    }
}

/// setfacl with `args` on each of `paths` at once.
fn acl_many(args: &[&str], paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    if !Command::new(SETFACL).args(args).arg("--").args(paths).status().context("running setfacl")?.success() {
        bail!("could not share {} (setfacl)", paths[0].display());
    }
    Ok(())
}

/// Share `dir` with `agent`: the agent may read and write what of it is the person's (`uid`, `me`);
/// what it makes there, the person may rename and delete, and (default ACLs) read and write unless
/// the agent narrows it — the agent owns what it makes. What the agent made is not the person's to set
/// ACLs on. No link is followed.
fn share_acls(dir: &Path, uid: u32, agent: &str, me: &str) -> Result<()> {
    let (dirs, files) = owned(dir, uid)?;
    for chunk in files.chunks(200) {
        acl_many(&["-m", &format!("u:{agent}:rwX")], chunk)?;
    }
    for chunk in dirs.chunks(200) {
        acl_many(&["-m", &format!("u:{agent}:rwx")], chunk)?;
        acl_many(&["-d", "-m", &format!("u:{agent}:rwX,u:{me}:rwX")], chunk)?;
    }
    Ok(())
}

/// Whether `me` is an administrator, as sudo will see it: in the group `wheel`, by /etc/group's list of its
/// members or as their own first group in /etc/passwd. The launcher's sudo rule is that group's
/// (modules/agent.nix): this only says so before sudo would, in words; who may is sudo's to decide.
pub(crate) fn administrator(group: &str, passwd: &str, me: &str) -> bool {
    let Some(wheel) = group.lines().map(|l| l.split(':').collect::<Vec<_>>()).find(|f| f.first() == Some(&"wheel")) else { return false };
    let listed = wheel.get(3).is_some_and(|members| members.split(',').any(|m| m == me));
    let first = passwd.lines().map(|l| l.split(':').collect::<Vec<_>>()).find(|f| f.first() == Some(&me)).and_then(|f| f.get(3).copied());
    listed || (first.is_some() && first == wheel.get(2).copied())
}

/// What someone who is not an administrator is told, before sudo would ask them for a password it then refuses.
pub(crate) const NOT_AN_ADMINISTRATOR: &str = "Only an administrator of this machine can start a program as their agent today, and you are not one: ask an administrator.";

/// Who starts a program as their agent: the person, their agent account, and the launcher. Refused, each
/// in words, for an agent's own account, someone with no agent, and someone who is not an administrator.
struct Caller {
    me: String,
    agent: String,
    launcher: String,
}

fn caller() -> Result<Caller> {
    let me = crate::me()?;
    if is_agent(&me) {
        bail!("this is an agent's account already ({me}): only a person starts a program as their agent");
    }
    // Your own agent, and only yours: the launcher runs `torq-agent-<you>` (as sudo says who you are).
    let agent = agent_of(&me);
    has_agent(&agent)?;
    let group = std::fs::read_to_string("/etc/group").context("reading /etc/group")?;
    let passwd = std::fs::read_to_string("/etc/passwd").context("reading /etc/passwd")?;
    if !administrator(&group, &passwd, &me) {
        bail!("{NOT_AN_ADMINISTRATOR}");
    }
    // The launcher by its own path in the store, as the sudo rule names it (sudo does not take a link
    // to it for it).
    let launcher = std::fs::canonicalize(LAUNCHER).with_context(|| format!("{LAUNCHER}: the agent's launcher is not installed"))?;
    Ok(Caller { me, agent, launcher: launcher.to_string_lossy().into_owned() })
}

/// Whether `acl` (what `getfacl` prints of a folder) has an entry for `agent`: the folder was shared with it before.
pub(crate) fn has_entry(acl: &str, agent: &str) -> bool {
    !agent.is_empty() && acl.lines().any(|l| l.strip_prefix("user:").and_then(|r| r.split(':').next()) == Some(agent))
}

/// Whether `dir` itself already has an entry for `agent` (asked of `getfacl`, the folder's own entries only).
/// Not known (getfacl could not be run): taken as not shared before, so the sharing is said.
fn shared_before(dir: &Path, agent: &str) -> bool {
    let mut c = Command::new(GETFACL);
    c.args(["-p", "-c", "--"]).arg(dir);
    setup::output_within(c, std::time::Duration::from_secs(10)).is_some_and(|acl| has_entry(&acl, agent))
}

/// `torq agent run …`: what was asked, then `launch`.
pub(crate) fn run(args: &[String]) -> Result<i32> {
    let r = parse(args)?;
    launch(&caller()?, r)
}

/// What `claude`, typed plainly, says where the folder the person is in is not shared.
pub(crate) fn nothing_shared(why: NotShared) -> String {
    format!("Nothing is shared with your agent here ({}): it works in its own folder. To let it work on a project, go into that project's folder first (cd ~/<folder>)", why.here())
}

/// `torq agent claude [<anything Claude Code takes> …]`, which is what `claude` typed plainly runs
/// (`torq agent plainCommand`): Claude Code as the person's agent, every word after it passed on unchanged.
/// The folder the person is in is shared with it, as `torq agent run --share <that folder> -- claude` does;
/// in a folder that is never shared (the home itself, outside it, a hidden folder of it) nothing is shared,
/// said in one line, and Claude Code runs in the agent's own folder — so `claude --version` works anywhere.
pub(crate) fn plain(args: &[String]) -> Result<i32> {
    let who = caller()?;
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set: no home to share from")?);
    let here = std::env::current_dir().map_err(|_| NotShared::Gone).and_then(|d| shareable(&d, &home, rustix::process::getuid().as_raw()));
    let share = here.map_err(|why| eprintln!("{}", nothing_shared(why))).ok();
    let standing = instructions::mine();
    if let Some(why) = standing.unusable() {
        eprintln!("{why}");
    }
    launch(&who, Run { share, command: plain_command(standing.text(), args) })
}

/// What `claude`, typed plainly, runs as the agent: Claude Code, given the person's standing instructions
/// (`--append-system-prompt`, under a line that says whose words they are) before the person's own words, which
/// follow unchanged. Before them, so that an `--append-system-prompt` of the person's own, later on the line,
/// is the one Claude Code takes. (`torq agent run -- claude` gives no instructions: it runs what it is told.)
pub(crate) fn plain_command(instructions: Option<&str>, args: &[String]) -> Vec<String> {
    let mut command = vec!["claude".to_string()];
    if let Some(text) = instructions {
        command.extend(["--append-system-prompt".to_string(), torq_wire::agent::standing(text)]);
    }
    command.extend(args.iter().cloned());
    command
}

/// `torq agent instructions [edit | clear]`: the person's standing instructions for their agent
/// (`instructions`): where they are and what they say; `edit` opens them in the person's editor (`$VISUAL`,
/// `$EDITOR`, else nano, else vi), making the file first; `clear` removes them.
pub(crate) fn instructions(args: &[String]) -> Result<i32> {
    use std::io::IsTerminal;
    const USAGE: &str = "usage: torq agent instructions (shows them) | torq agent instructions edit | torq agent instructions clear";
    if is_agent(&crate::me()?) {
        bail!("this is an agent's account: a person's instructions for their agent are that person's to write, in their own home");
    }
    let file = instructions::path().context("HOME is not set: no home to keep your instructions in")?;
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => print!("{}", instructions::shown(&file, &instructions::read(&file))),
        ["clear"] => {
            let was = instructions::clear(&file).with_context(|| format!("removing {}", file.display()))?;
            println!("{}", if was { "Your instructions for your agent are removed. It is told nothing of them from your next question on." } else { "There were no instructions for your agent to remove." });
        }
        ["edit"] => {
            if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
                bail!("torq agent instructions edit opens your editor: at a terminal. The file is {} (plain text: any editor will do)", file.display());
            }
            instructions::create(&file).with_context(|| format!("making {}", file.display()))?;
            let editor = ["VISUAL", "EDITOR"].iter().find_map(|v| std::env::var(v).ok().filter(|e| !e.trim().is_empty()));
            let editor = editor.unwrap_or_else(|| if crate::shell::which("nano") { "nano".into() } else { "vi".into() });
            // As the shell would run $EDITOR (it may carry options: `code --wait`).
            let status = Command::new("sh").arg("-c").arg(format!("{editor} \"$1\"")).arg("sh").arg(&file).status().with_context(|| format!("starting your editor ({editor})"))?;
            if !status.success() {
                bail!("the editor ({editor}) ended with an error; the file is {}", file.display());
            }
            match instructions::read(&file) {
                instructions::Instructions::None => println!("No instructions for your agent (the file is empty)."),
                instructions::Instructions::Text(_) => println!("Saved. Your agent is given them from your next question on, and when you next type `claude`."),
                instructions::Instructions::Unusable(why) => bail!("{why}"),
            }
        }
        _ => bail!("{USAGE}"),
    }
    Ok(0)
}

/// Run it: share the directory (ACLs, and the directory itself, open), then become the agent,
/// through the launcher. Does not return on success: the program takes this process's place.
fn launch(who: &Caller, r: Run) -> Result<i32> {
    use std::os::fd::AsRawFd;
    let Caller { me, agent, launcher } = who;
    let mut sudo: Vec<String> = Vec::new();
    let mut launch = vec![launcher.clone()];
    // Held open until the program has taken this process's place: the launcher binds it.
    let mut _shared: Option<std::fs::File> = None;
    if let Some(share) = &r.share {
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set: no home to share from")?);
        let uid = rustix::process::getuid().as_raw();
        let dir = checked_share(share, &home, uid)?;
        // Said only the first time: a folder that already has the agent's entry was shared before, and the
        // walk below only brings what is new in it under the same sharing.
        let before = shared_before(&dir, agent);
        // One session of the agent's at a time, and none of its programs running while the directory is
        // walked (a link swapped in between the walk's look and its ACL would be granted wherever it
        // leads): the person's agent service held still by systemd's freezer, any other program of the
        // agent's refused — both by the launcher, as root — and thawed after, however the walk ended.
        // With its own process namespace (the launcher), nothing of a session outlives it.
        let frozen = Frozen::hold(&launch[0])?;
        let shared = share_acls(&dir, uid, agent, me);
        // Thawed whatever the walk did, and both said when both fail (the
        // walk's own error was lost behind the thaw's).
        let thawed = frozen.thaw();
        match (shared, thawed) {
            (Err(e), Err(t)) => return Err(e.context(format!("and your agent could not be thawed after: {t:#}"))),
            (Err(e), Ok(())) | (Ok(()), Err(e)) => return Err(e),
            (Ok(()), Ok(())) => {}
        }
        // Opened as the person, kept open across exec (no close-on-exec), and kept by sudo (`-C`,
        // which the launcher's rule allows): the agent is handed the directory, not a way to it.
        let f = std::fs::File::open(&dir).with_context(|| format!("opening {}", dir.display()))?;
        rustix::io::fcntl_setfd(&f, rustix::io::FdFlags::empty()).context("keeping the shared directory open")?;
        let fd = f.as_raw_fd();
        sudo.extend(["-C".into(), (fd + 1).to_string()]);
        launch.extend(["--share-fd".into(), fd.to_string(), dir.to_string_lossy().into_owned()]);
        _shared = Some(f);
        if !before {
            eprintln!("{}", shared_line(&dir, agent));
        }
    }
    launch.push("--".into());
    launch.extend(r.command);
    use std::os::unix::process::CommandExt;
    let e = Command::new(SUDO).args(&sudo).arg("--").args(&launch).exec();
    Err(anyhow::Error::from(e).context("starting the agent (sudo torq-agent-run)"))
}

/// Said the first time a folder is shared: what the agent may do there now, and how to take it back.
pub(crate) fn shared_line(dir: &Path, agent: &str) -> String {
    format!("Shared with your agent ({agent}): {} — it can now read and change what is in this folder, and nothing else of yours. To take that back: setfacl -R -P -x u:{agent},d:u:{agent} -- {}", dir.display(), dir.display())
}

/// What `torq ask` was asked: a question for the person's agent, or to forget the conversation.
#[derive(Debug, PartialEq)]
pub(crate) enum Asked {
    Question(String),
    Forget,
    /// No words: the question is one line read from standard input, so it needs no quoting.
    FromInput,
}

const ASK_USAGE: &str = "usage: torq ask \"<question>\" | torq ask (then type the question) | torq ask --forget — e.g. torq ask \"why is my wifi flaky?\"";

/// `ask "<question>"`, `ask -- <words…>`, `ask --forget`, or `ask` alone (the question from standard input).
pub(crate) fn parse_ask(args: &[String]) -> Result<Asked> {
    let words = match args {
        [] => return Ok(Asked::FromInput),
        [f] if f == "--forget" => return Ok(Asked::Forget),
        [d, rest @ ..] if d == "--" => rest,
        [first, ..] if first.starts_with('-') => bail!("unknown option {first}\n{ASK_USAGE}"),
        rest => rest,
    };
    let q = words.join(" ");
    if q.trim().is_empty() {
        bail!("{ASK_USAGE}");
    }
    Ok(Asked::Question(q))
}

/// One line read as the question (`ask` alone): whatever it holds, no shell reads it. At most 8 KiB: a longer
/// one is refused, never cut unsaid.
fn question_from<R: std::io::BufRead>(r: &mut R) -> Result<String> {
    use std::io::Read;
    const MAX: u64 = 8 * 1024;
    let mut line = String::new();
    std::io::BufRead::read_line(&mut r.take(MAX + 1), &mut line).context("reading your question")?;
    if line.trim_end_matches('\n').len() as u64 > MAX {
        bail!("your question is longer than {MAX} bytes: ask it shorter");
    }
    let q = line.trim();
    if q.is_empty() {
        bail!("{ASK_USAGE}");
    }
    Ok(q.to_string())
}

/// What a key does while an answer comes.
#[derive(Debug, PartialEq)]
enum KeyDoes {
    Nothing,
    /// The first Ctrl-C: ask the agent to stop.
    Stop,
    /// A second: leave, whatever the agent does.
    Leave,
}

/// How long after a Ctrl-C the agent has to end its task before `torq ask` leaves without it.
const GIVE_UP: std::time::Duration = std::time::Duration::from_secs(5);

/// Ctrl-C asks the agent to stop; a second Ctrl-C, or no end within `GIVE_UP` of the first, gets the
/// person out (a hung or frozen service never holds the terminal).
#[derive(Default)]
struct Interrupt {
    stopped_at: Option<std::time::Instant>,
}

impl Interrupt {
    fn key(&mut self, b: u8, now: std::time::Instant) -> KeyDoes {
        match (b, self.stopped_at) {
            (0x03, None) => {
                self.stopped_at = Some(now);
                KeyDoes::Stop
            }
            (0x03, Some(_)) => KeyDoes::Leave,
            _ => KeyDoes::Nothing,
        }
    }

    fn given_up(&self, now: std::time::Instant) -> bool {
        self.stopped_at.is_some_and(|t| now.duration_since(t) >= GIVE_UP)
    }
}

/// Ctrl-C, while an answer comes: the terminal's keys read as keys (no signal, no echo), the first
/// Ctrl-C a `stop` sent, a second — or no end within `GIVE_UP` — the terminal as it was and exit 130;
/// the terminal as it was, too, when this ends, and when a signal would end it (`Signals`).
struct Keys {
    was: rustix::termios::Termios,
    /// Whether the person stopped the task (Ctrl-C): `torq ask` then ends 130, as when they leave.
    stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// The signals that would end `torq ask` with the terminal still in its key mode:
/// blocked before any thread starts, so that every later thread has them blocked too, and read from a
/// signalfd instead. One that comes puts the terminal back and ends as its signal would (128 + its number).
struct Signals(nix::sys::signalfd::SignalFd);

impl Signals {
    fn hold() -> Option<Signals> {
        use nix::sys::signal::{SigSet, Signal};
        let mut set = SigSet::empty();
        for s in [Signal::SIGTERM, Signal::SIGHUP, Signal::SIGINT, Signal::SIGQUIT] {
            set.add(s);
        }
        set.thread_block().ok()?;
        match nix::sys::signalfd::SignalFd::new(&set) {
            Ok(fd) => Some(Signals(fd)),
            // Held and nothing to read them from: let go again, so they end it as they would.
            Err(_) => {
                let _ = set.thread_unblock();
                None
            }
        }
    }

    /// The next of them (blocking), its number.
    fn next(&self) -> Option<i32> {
        self.0.read_signal().ok().flatten().map(|i| i.ssi_signo as i32)
    }
}

/// Held and then not watched (the terminal could not be read): let go again, so the signals end it as they would.
impl Drop for Signals {
    fn drop(&mut self) {
        use nix::sys::signal::{SigSet, Signal};
        let mut set = SigSet::empty();
        for s in [Signal::SIGTERM, Signal::SIGHUP, Signal::SIGINT, Signal::SIGQUIT] {
            set.add(s);
        }
        // Nothing to do if it fails: the signals stay held, read by no one, as they were.
        let _ = set.thread_unblock();
    }
}

impl Keys {
    fn watch(agent: client::Client, signals: Option<Signals>) -> Option<Keys> {
        use rustix::termios::{tcgetattr, tcsetattr, LocalModes, OptionalActions, SpecialCodeIndex};
        let stdin = std::io::stdin();
        let was = tcgetattr(&stdin).ok()?;
        let mut keys = was.clone();
        keys.local_modes.remove(LocalModes::ICANON | LocalModes::ECHO | LocalModes::ISIG);
        keys.special_codes[SpecialCodeIndex::VMIN] = 1;
        keys.special_codes[SpecialCodeIndex::VTIME] = 0;
        tcsetattr(&stdin, OptionalActions::Now, &keys).ok()?;
        if let Some(signals) = signals {
            let restore = was.clone();
            std::thread::spawn(move || {
                if let Some(n) = signals.next() {
                    // Putting it back may fail (the terminal gone, as at a hangup): then there is nothing to put back.
                    let _ = rustix::termios::tcsetattr(std::io::stdin(), rustix::termios::OptionalActions::Now, &restore);
                    // Said if it can be: at a hangup the terminal is gone, and eprintln! would panic this thread and
                    // leave torq ask running, the signal taken. Whatever the write does, it ends.
                    let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("agent: ended by signal {n}, without its answer (your agent stops answering when the terminal that asked is gone)\n"));
                    std::process::exit(128 + n);
                }
            });
        }
        let restore = was.clone();
        let stopped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let said_stop = stopped.clone();
        std::thread::spawn(move || {
            use rustix::event::{poll, PollFd, PollFlags, Timespec};
            let (mut i, mut open) = (Interrupt::default(), true);
            loop {
                let stdin = std::io::stdin();
                let mut fds = [PollFd::new(&stdin, PollFlags::IN)];
                let ready = open && poll(&mut fds, Some(&Timespec { tv_sec: 0, tv_nsec: 200_000_000 })).is_ok_and(|n| n > 0);
                if !open {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                let mut does = KeyDoes::Nothing;
                if ready {
                    let mut b = [0u8; 1];
                    match rustix::io::read(&stdin, &mut b) {
                        Ok(1) => does = i.key(b[0], std::time::Instant::now()),
                        _ => open = false,
                    }
                }
                if does == KeyDoes::Stop {
                    eprintln!("agent: stopping (Ctrl-C again to leave now)");
                    said_stop.store(true, std::sync::atomic::Ordering::Relaxed);
                    // On a thread of its own: the service answers a stop once the task is over, and a second
                    // Ctrl-C must be read meanwhile. A stop not taken is said.
                    let agent = agent.clone();
                    std::thread::spawn(move || {
                        if let Err(e) = agent.stop() {
                            eprintln!("agent: the stop was not taken: {e:#}");
                        }
                    });
                }
                if does == KeyDoes::Leave || i.given_up(std::time::Instant::now()) {
                    let _ = rustix::termios::tcsetattr(&stdin, rustix::termios::OptionalActions::Now, &restore);
                    eprintln!("agent: left without its answer (it may still be stopping)");
                    std::process::exit(130);
                }
            }
        });
        Some(Keys { was, stopped })
    }
}

impl Drop for Keys {
    fn drop(&mut self) {
        let _ = rustix::termios::tcsetattr(std::io::stdin(), rustix::termios::OptionalActions::Now, &self.was);
    }
}

/// What a person reads where a proposal of their agent's is told (`torq ask`, the window, the applet). The
/// broker marks every proposal of a coding tool's tainted, whatever it read; what that means to the person is this.
pub const ALWAYS_ASKED: &str = "Your agent's proposals always wait for you to read and approve them.";

/// What Forget did, as every program says it.
pub const FORGOTTEN: &str = "The conversation is deleted. Your next question starts a new one.";

/// What Forget will do, asked before it is done (the window's and the Agent page's question).
pub const FORGET_ASKS: &str = "The conversation is deleted, from your agent's account too. Your next question starts a new one. What your agent already proposed or changed stays.";

/// Said in a window that was showing a conversation deleted from somewhere else.
pub const FORGOTTEN_ELSEWHERE: &str = "The conversation was deleted (Forget, from another window or a terminal). Your next question starts a new one.";

/// What became of proposal `seq`, by the ledger: only one of the person's own agent's is theirs. The ledger
/// unread is said as that, never taken as a yes.
fn looked_up(seq: u64) -> conversation::Fate {
    crate::app::agent_fates(&[seq]).ok().and_then(|f| f.first().map(|(_, fate)| *fate)).unwrap_or(conversation::Fate::Unread)
}

/// How the person reads an event: the answer and the proposals on standard output, the steps on the
/// way on standard error, every word of the agent's cleaned of what could fool a terminal. `Some`
/// (the exit code) when it is the last — an error is the last, too, when `error_ends` (for `forget`).
///
/// A proposal's number is the service's word, and so that of anything the engine ran (it can write the line
/// the service reads the number from): `fate` looks it up in the ledger, as the agent's window does, and only
/// a proposal of the person's own agent that waits is shown as one to apply.
fn show(e: torq_wire::agent::Event, error_ends: bool, failed: &mut bool, fate: &dyn Fn(u64) -> conversation::Fate, out: &mut dyn std::io::Write, err: &mut dyn std::io::Write) -> std::io::Result<Option<i32>> {
    use crate::history::clean;
    use conversation::Fate;
    use torq_wire::agent::Event;
    match e {
        Event::Say { text } => {
            for line in text.lines() {
                writeln!(out, "{}", clean(line))?;
            }
        }
        Event::Called { tool } => writeln!(err, "agent: {}", clean(&tool))?,
        // Why the broker marked it (its own word, `coding-tool`) is not a person's to read: what it means is.
        Event::Tainted { .. } => writeln!(err, "agent: {ALWAYS_ASKED}")?,
        Event::Proposed { seq } => match fate(seq) {
            Fate::Waiting => writeln!(out, "Proposed #{seq}: it waits for you. To read it: `show system proposals`. To apply it: `request system apply {seq}`")?,
            Fate::Failed => writeln!(out, "Proposed #{seq}: applying it failed once, so it waits for you again. To read it: `show system proposals`. To apply it: `request system apply {seq}`")?,
            Fate::Applied => writeln!(out, "Proposed #{seq}: already applied (`show system ledger` has it)")?,
            Fate::Declined => writeln!(out, "Proposed #{seq}: already declined")?,
            Fate::NotYours => writeln!(err, "agent: your agent said it made proposal #{seq}, but that is not your agent's proposal: there is nothing to apply. `show system proposals` lists what really waits for you")?,
            Fate::Unread => writeln!(err, "agent: your agent said it made proposal #{seq}; the record of changes could not be read to check that. `show system proposals` lists what really waits for you")?,
        },
        Event::Error { text } => {
            *failed = true;
            writeln!(err, "agent: {}", clean(&text))?;
            if error_ends {
                return Ok(Some(1));
            }
        }
        Event::Done { exit } => return Ok(Some(if exit == 0 && !*failed { 0 } else { 1 })),
        Event::Forgotten => {
            writeln!(out, "{FORGOTTEN}")?;
            return Ok(Some(0));
        }
        Event::Status { .. } => {}
    }
    Ok(None)
}

/// `torq ask "<question>"` (ADR-051 4, the terminal's way to the agent): the person's own agent —
/// its service, `torq-agent@<you>.service`, on its socket — asked through the client every UI shares
/// (`client::Client`), the answer streamed as it comes, with the number of anything it proposed; Ctrl-C
/// stops it. `--forget`: the conversation is deleted (the engine's own record of it too), and the next question
/// starts a new one.
pub(crate) fn ask(args: &[String]) -> Result<i32> {
    use torq_wire::agent::Event;
    let asked = parse_ask(args)?;
    let agent = client::Client::mine()?;
    let text = match asked {
        Asked::Forget => {
            return match agent.forget() {
                Ok(()) => show(Event::Forgotten, true, &mut false, &looked_up, &mut std::io::stdout(), &mut std::io::stderr()).map(|c| c.unwrap_or(0)).map_err(Into::into),
                Err(client::Unreachable::Refused(text)) => show(Event::Error { text }, true, &mut false, &looked_up, &mut std::io::stdout(), &mut std::io::stderr()).map(|c| c.unwrap_or(1)).map_err(Into::into),
                Err(e) => Err(e.into()),
            };
        }
        Asked::Question(text) => text,
        Asked::FromInput => {
            use std::io::IsTerminal;
            if std::io::stdin().is_terminal() {
                eprint!("Ask your agent: ");
            }
            question_from(&mut std::io::stdin().lock())?
        }
    };
    // Held before any thread starts (a thread started earlier would take them, and end with the terminal raw), and
    // only now: at the question's prompt, Ctrl-C ends torq ask as it always does. Until the key thread reads them (just
    // below) they are held unread for as long as agent.ask takes to return its stream: bounded, its connect and its
    // request each within the client's QUICK (15 s).
    use std::io::IsTerminal;
    let signals = std::io::stdin().is_terminal().then(Signals::hold).flatten();
    let mut answer = agent.ask(&text, None)?;
    // Instructions of the person's that could not be used: said once, beside the answer (it was asked without them).
    if let Some(note) = answer.note() {
        eprintln!("agent: {note}");
    }
    let keys = if std::io::stdin().is_terminal() { Keys::watch(agent.clone(), signals) } else { None };
    // Stopped by the person (Ctrl-C): 130, as when they leave, whatever the task's own end.
    let stopped = || keys.as_ref().is_some_and(|k| k.stopped.load(std::sync::atomic::Ordering::Relaxed));
    let mut failed = false;
    for e in answer {
        // An error, then the end: said already, so only the code (as the service closed without its `done`).
        let e = match e {
            Err(_) if failed => return Ok(if stopped() { 130 } else { 1 }),
            e => e?,
        };
        if let Some(code) = show(e, false, &mut failed, &looked_up, &mut std::io::stdout(), &mut std::io::stderr())? {
            return Ok(if stopped() { 130 } else { code });
        }
    }
    bail!("your agent's service ended before it answered (journalctl -u {}, an administrator's)", agent.unit())
}

/// `torq doctor "<question>"` (ADR-042): the agent, in doctor mode, asked one question — run as the
/// agent's account through `torq agent run` (no directory shared), so it reads only what the agent
/// may, and proposes nothing. As the agent already (a tool run as it), the doctor itself.
///
/// `torq agent propose "<ask>"` (`propose`; ADR-042, 6): the same, with one more tool — it may
/// propose a change within what the agent may (torq.agent.mayPropose), never apply one: a person
/// applies it with `request system apply <n>`.
pub(crate) fn doctor_mode(args: &[String], propose: bool) -> Result<i32> {
    const DOCTOR: &str = "/run/current-system/sw/bin/torq-doctor";
    let question = args.join(" ");
    if question.trim().is_empty() {
        bail!(if propose {
            "usage: torq agent propose \"<what you want changed>\" — e.g. torq agent propose \"install htop for me\""
        } else {
            "usage: torq doctor \"<question>\" — e.g. torq doctor \"why is my wifi flaky?\""
        });
    }
    if !Path::new(DOCTOR).exists() {
        bail!("doctor mode is off on this machine: configure; set torq agent doctor enable true; commit (ADR-042)");
    }
    let mut doctor: Vec<String> = vec![DOCTOR.into()];
    if propose {
        doctor.push("--propose".into());
    }
    doctor.extend(["--".into(), question]);
    if is_agent(&crate::me()?) {
        use std::os::unix::process::CommandExt;
        let e = Command::new(&doctor[0]).args(&doctor[1..]).exec();
        return Err(anyhow::Error::from(e).context("starting the doctor"));
    }
    run(&[vec!["run".to_string(), "--".to_string()], doctor].concat())
}

/// `torq break-glass "<reason>"` (ADR-035): a root shell, after the record. The launcher, through
/// sudo (the person's password), writes the ledger entry, pushes it, and only then opens the shell.
pub(crate) fn break_glass(args: &[String]) -> Result<i32> {
    const LAUNCHER: &str = "/run/current-system/sw/bin/torq-break-glass";
    let reason = args.join(" ");
    if reason.trim().is_empty() {
        bail!("usage: torq break-glass \"<why>\" — the reason goes in the ledger, before the shell opens");
    }
    if is_agent(&crate::me()?) {
        bail!("the agent has no root shell: it proposes, and a person applies");
    }
    let launcher = std::fs::canonicalize(LAUNCHER).with_context(|| format!("{LAUNCHER}: not on this machine"))?;
    use std::os::unix::process::CommandExt;
    let e = Command::new(SUDO).arg("--").arg(launcher).arg(&reason).exec();
    Err(anyhow::Error::from(e).context("starting break-glass (sudo)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    /// ADR-051 3: each person's agent is `torq-agent-<person>`; the old single account, a bare prefix
    /// and a person are not named as one (torqcli's convenience; the broker's map is the authority).
    #[test]
    fn whose_agent_by_name() {
        assert_eq!(agent_of("alice"), "torq-agent-alice");
        assert!(is_agent("torq-agent-alice") && is_agent(&agent_of("bob")) && is_agent("torq-resident-alice"));
        for not in ["torq-agent", "torq-agent-", "torq-resident-", "torq-resident", "alice", "xtorq-agent-alice"] {
            assert!(!is_agent(not), "{not}");
        }
    }

    #[test]
    fn what_it_is_asked() {
        assert_eq!(parse(&a("run")).unwrap(), Run { share: None, command: vec![] });
        assert_eq!(parse(&a("run claude")).unwrap(), Run { share: None, command: a("claude") });
        assert_eq!(parse(&a("run --share /home/v/p -- claude --resume")).unwrap(), Run { share: Some("/home/v/p".into()), command: a("claude --resume") });
        assert_eq!(parse(&a("run -- -x")).unwrap().command, a("-x"), "after --, anything");
        assert!(parse(&a("run --share")).is_err());
        assert!(parse(&a("run --shar /x")).is_err(), "no other option");
        assert!(parse(&a("start")).is_err());
    }

    #[test]
    fn the_doctor_needs_a_question() {
        for q in [vec![], vec!["  ".to_string()]] {
            assert!(format!("{:#}", doctor_mode(&q, false).unwrap_err()).starts_with("usage: torq doctor"));
            assert!(format!("{:#}", doctor_mode(&q, true).unwrap_err()).starts_with("usage: torq agent propose"));
        }
    }

    #[test]
    fn only_a_directory_of_ones_own_inside_ones_home() {
        use std::os::unix::fs::MetadataExt;
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        let project = home.join("src/project");
        std::fs::create_dir_all(&project).unwrap();
        let uid = std::fs::metadata(&home).unwrap().uid();
        assert_eq!(checked_share(&project, &home, uid).unwrap(), std::fs::canonicalize(&project).unwrap());
        // Each refusal says why, in words for the person who named the folder.
        let refused = |dir: &Path, uid: u32| checked_share(dir, &home, uid).unwrap_err().to_string();
        let own = refused(&home, uid);
        assert!(own.contains("is your home folder itself") && own.contains("share a folder inside it"), "{own}");
        let outside = refused(d.path(), uid);
        assert!(outside.contains("is not inside your home"), "{outside}");
        let anothers = refused(&project, uid + 1);
        assert!(anothers.contains("not yours to share"), "{anothers}");
        std::fs::write(home.join("file"), "").unwrap();
        assert_eq!(shareable(&home.join("file"), &home, uid), Err(NotShared::NotADirectory), "not a file");
        assert_eq!(shareable(&home.join("nowhere"), &home, uid), Err(NotShared::Gone));
        // A link out of the home is where it leads, and refused there.
        std::os::unix::fs::symlink(d.path(), home.join("out")).unwrap();
        assert_eq!(shareable(&home.join("out"), &home, uid), Err(NotShared::Outside));
        // None of them tells the person to "name one with --share": they did, or `claude` did for them.
        for said in [&own, &outside, &anothers] {
            assert!(!said.contains("--share"), "{said}");
        }
    }

    /// A hidden folder directly under the home, and anything inside one, is never shared: keys and settings
    /// live there. A hidden folder inside a project is the project's.
    #[test]
    fn a_hidden_folder_of_the_home_is_never_shared() {
        use std::os::unix::fs::MetadataExt;
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("home");
        for dir in [".ssh", ".config/x/deep", ".local/share", "project/.git/hooks", "project/sub", "not.hidden"] {
            std::fs::create_dir_all(home.join(dir)).unwrap();
        }
        let uid = std::fs::metadata(&home).unwrap().uid();
        for hidden in [".ssh", ".config", ".config/x", ".config/x/deep", ".local/share"] {
            assert_eq!(shareable(&home.join(hidden), &home, uid), Err(NotShared::Hidden), "{hidden}");
        }
        let said = checked_share(&home.join(".ssh"), &home, uid).unwrap_err().to_string();
        assert!(said.contains("is a hidden folder of your home") && said.contains("Your keys, sign-ins and settings live there") && said.contains("share a folder of your own work instead"), "{said}");
        // A link to one, under a plain name, is where it leads.
        std::os::unix::fs::symlink(home.join(".ssh"), home.join("keys")).unwrap();
        assert_eq!(shareable(&home.join("keys"), &home, uid), Err(NotShared::Hidden));
        // The controls: a project, a hidden folder inside it, a name with a dot elsewhere in it.
        for fine in ["project", "project/sub", "project/.git", "project/.git/hooks", "not.hidden"] {
            assert_eq!(shareable(&home.join(fine), &home, uid), Ok(std::fs::canonicalize(home.join(fine)).unwrap()), "{fine}");
        }
    }

    /// `claude`, typed plainly, where the folder is not shared: one line, why, and what to do. It never says
    /// "--share": the person typed `claude`.
    #[test]
    fn what_the_plain_command_says_where_nothing_is_shared() {
        assert_eq!(
            nothing_shared(NotShared::Home),
            "Nothing is shared with your agent here (this is your home folder itself): it works in its own folder. To let it work on a project, go into that project's folder first (cd ~/<folder>)"
        );
        for why in [NotShared::Gone, NotShared::NotADirectory, NotShared::Home, NotShared::Outside, NotShared::Hidden, NotShared::NotYours] {
            let said = nothing_shared(why);
            assert!(said.starts_with("Nothing is shared with your agent here (") && said.contains("go into that project's folder first") && !said.contains("--share") && !said.contains('\n'), "{said}");
        }
        assert!(nothing_shared(NotShared::Hidden).contains("a hidden folder of your home, where keys and settings live"));
        assert!(nothing_shared(NotShared::Outside).contains("outside your home"));
    }

    /// `claude`, typed plainly: Claude Code, given the person's standing instructions before their own words,
    /// which follow as they typed them.
    #[test]
    fn the_plain_command_gives_the_instructions_before_the_persons_words() {
        let words = a("--model opus -p hi");
        assert_eq!(plain_command(None, &words), a("claude --model opus -p hi"), "none written: Claude Code and the person's words");
        let with = plain_command(Some("Always answer in German.\n"), &words);
        assert_eq!(with[..3], ["claude".to_string(), "--append-system-prompt".into(), "[Standing instructions from the person this agent works for]\nAlways answer in German.".into()]);
        assert_eq!(with[3..], words[..], "then every word as typed");
        // The person's own, later on the line, stays after TorqOS's.
        let own = plain_command(Some("x"), &a("--append-system-prompt mine"));
        assert_eq!(own.iter().filter(|w| *w == "--append-system-prompt").count(), 2);
        assert_eq!(own.last().map(String::as_str), Some("mine"));
        assert_eq!(plain_command(Some("x"), &[]).len(), 3);
    }

    /// Who is an administrator, as sudo's rule for the launcher sees it: a member of wheel, listed or by
    /// their first group.
    #[test]
    fn who_is_an_administrator() {
        let group = "root:x:0:\nwheel:x:1:alice,dave\nusers:x:100:\nwheelers:x:5:bob\n";
        let passwd = "alice:x:1000:100::/home/alice:/bin/sh\nbob:x:1001:100::/home/bob:/bin/sh\ncarol:x:1002:1::/home/carol:/bin/sh\n";
        assert!(administrator(group, passwd, "alice") && administrator(group, passwd, "dave"));
        assert!(administrator(group, passwd, "carol"), "wheel is her first group");
        assert!(!administrator(group, passwd, "bob"), "a group that only begins so, and listed in no wheel");
        assert!(!administrator(group, passwd, "ali") && !administrator(group, passwd, ""));
        assert!(!administrator("users:x:100:alice\n", passwd, "alice"), "no wheel group at all");
        assert!(NOT_AN_ADMINISTRATOR.starts_with("Only an administrator") && NOT_AN_ADMINISTRATOR.ends_with("ask an administrator."));
    }

    /// The long "Shared with your agent" line is for the first time only: a folder whose own entries already
    /// name the agent was shared before.
    #[test]
    fn a_folder_shared_before_is_known_by_its_entry() {
        let shared = "user::rwx\nuser:torq-agent-alice:rwx\ngroup::r-x\nmask::rwx\nother::r-x\ndefault:user::rwx\ndefault:user:torq-agent-alice:rwx\n";
        assert!(has_entry(shared, "torq-agent-alice"));
        assert!(!has_entry(shared, "torq-agent-al") && !has_entry(shared, "torq-agent-alice2") && !has_entry(shared, ""));
        assert!(!has_entry("user::rwx\ngroup::r-x\nother::r-x\n", "torq-agent-alice"), "never shared");
        assert!(!has_entry("user::rwx\ndefault:user:torq-agent-alice:rwx\ngroup:torq-agent-alice:rwx\n", "torq-agent-alice"), "only the folder's own entry for the account");
        let line = shared_line(Path::new("/home/alice/work"), "torq-agent-alice");
        assert!(line.starts_with("Shared with your agent (torq-agent-alice): /home/alice/work") && line.contains("To take that back: setfacl -R -P -x u:torq-agent-alice,d:u:torq-agent-alice -- /home/alice/work"), "{line}");
    }

    #[test]
    fn the_walk_takes_only_ones_own_and_follows_no_link() {
        use std::os::unix::fs::MetadataExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("p");
        std::fs::create_dir_all(p.join("sub")).unwrap();
        std::fs::write(p.join("sub/f"), "").unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "").unwrap();
        std::os::unix::fs::symlink(outside.path(), p.join("link")).unwrap();
        let uid = std::fs::metadata(&p).unwrap().uid();
        let (dirs, files) = owned(&p, uid).unwrap();
        assert_eq!(dirs.len(), 2, "{dirs:?}");
        assert_eq!(files, [p.join("sub/f")]);
        assert_eq!(owned(&p, uid + 1).unwrap(), (vec![], vec![]), "someone else's: nothing");
    }


    #[test]
    fn what_torq_ask_is_asked() {
        assert_eq!(parse_ask(&a("why is it slow?")).unwrap(), Asked::Question("why is it slow?".into()));
        assert_eq!(parse_ask(&a("--forget")).unwrap(), Asked::Forget);
        assert_eq!(parse_ask(&a("-- --forget me")).unwrap(), Asked::Question("--forget me".into()), "after --, words");
        assert_eq!(parse_ask(&a("forget --forget")).unwrap(), Asked::Question("forget --forget".into()));
        assert_eq!(parse_ask(&[]).unwrap(), Asked::FromInput, "no words: the question from standard input");
        let mut typed = std::io::Cursor::new("why is my wifi slow? it's $HOME * \"odd\"\nnext line\n");
        assert_eq!(question_from(&mut typed).unwrap(), "why is my wifi slow? it's $HOME * \"odd\"", "one line, as typed");
        assert!(question_from(&mut std::io::Cursor::new("\n")).is_err(), "nothing typed");
        for bad in ["--", "--stop", "-x why"] {
            assert!(parse_ask(&a(bad)).is_err(), "{bad:?}");
        }
        assert!(format!("{:#}", parse_ask(&a("--stop")).unwrap_err()).contains("unknown option --stop"));
    }

    /// The answer and the proposals on standard output, the steps on standard error, nothing of the
    /// agent's able to fool a terminal; the end, and its code, at `done`.
    #[test]
    fn how_the_answer_is_shown() {
        use conversation::Fate;
        use torq_wire::agent::Event;
        let (mut out, mut err, mut failed) = (Vec::new(), Vec::new(), false);
        let waiting = |_| Fate::Waiting;
        let mut s = |e| show(e, false, &mut failed, &waiting, &mut out, &mut err).unwrap();
        assert_eq!(s(Event::Called { tool: "read_journal".into() }), None);
        assert_eq!(s(Event::Tainted { why: "coding-tool\u{1b}[2J".into() }), None);
        assert_eq!(s(Event::Say { text: "line one\nrewrite\rd \u{1b}[2J\u{202e}".into() }), None);
        assert_eq!(s(Event::Proposed { seq: 12 }), None);
        assert_eq!(s(Event::Done { exit: 0 }), Some(0));
        let (out, err) = (String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap());
        assert!(out.starts_with("line one\n") && out.contains("Proposed #12: it waits for you") && out.contains("request system apply 12"), "{out}");
        assert!(!out.contains('\u{1b}') && !out.contains('\r') && !out.contains('\u{202e}'), "{out:?}");
        // The broker's own word for the taint is not shown: what it means to the person is.
        assert_eq!(err, "agent: read_journal\nagent: Your agent's proposals always wait for you to read and approve them.\n");
        // An error, then done: failed, whatever the code.
        let (mut failed, mut sink) = (false, Vec::new());
        assert_eq!(show(Event::Error { text: "a task is running already".into() }, false, &mut failed, &waiting, &mut Vec::new(), &mut sink).unwrap(), None);
        assert_eq!(show(Event::Done { exit: 0 }, false, &mut failed, &waiting, &mut Vec::new(), &mut Vec::new()).unwrap(), Some(1));
        assert_eq!(show(Event::Done { exit: 137 }, false, &mut false, &waiting, &mut Vec::new(), &mut Vec::new()).unwrap(), Some(1), "stopped");
    }

    /// A proposal's number is the service's word, which anything the engine ran can forge: only a number
    /// the ledger has as a waiting proposal of the person's own agent is shown as one to apply.
    #[test]
    fn a_proposal_is_shown_only_if_the_ledger_has_it_as_ones_own() {
        use conversation::Fate;
        use torq_wire::agent::Event;
        let shown = |fate: Fate| {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            assert_eq!(show(Event::Proposed { seq: 41 }, false, &mut false, &|seq| if seq == 41 { fate } else { Fate::Waiting }, &mut out, &mut err).unwrap(), None);
            (String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
        };
        // The control: one that waits, of the person's own agent.
        let (out, err) = shown(Fate::Waiting);
        assert!(out.contains("request system apply 41") && err.is_empty(), "{out} / {err}");
        assert!(shown(Fate::Failed).0.contains("request system apply 41"));
        // Another's, or none at all: nothing on standard output, no command to apply it, and said.
        for (fate, says) in [(Fate::NotYours, "that is not your agent's proposal: there is nothing to apply"), (Fate::Unread, "could not be read to check that")] {
            let (out, err) = shown(fate);
            assert_eq!(out, "", "{fate:?}");
            assert!(err.contains("proposal #41") && err.contains(says) && err.contains("`show system proposals`") && !err.contains("request system apply"), "{fate:?}: {err}");
        }
        // Over already: said, with no command to apply it again.
        for (fate, says) in [(Fate::Applied, "already applied"), (Fate::Declined, "already declined")] {
            let (out, _) = shown(fate);
            assert!(out.contains(says) && !out.contains("request system apply"), "{fate:?}: {out}");
        }
        // By the ledger: a number that is another agent's, or a person's own commit, is not this agent's.
        let entries = crate::history::parse(concat!(
            r#"{"seq":40,"ts":"2026-11-15T12:00:00Z","kind":"proposal","actor":"agent","tier":"user","reason":"htop","data":{"agent":"torq-agent-bob"}}"#, "\n",
            r#"{"seq":41,"ts":"2026-11-15T12:01:00Z","kind":"proposal","actor":"agent","tier":"user","reason":"jq","data":{"agent":"torq-agent-alice"}}"#, "\n",
            r#"{"seq":42,"ts":"2026-11-15T12:02:00Z","kind":"applied","actor":"user","tier":"user","reason":"x","data":{}}"#, "\n",
        ))
        .unwrap();
        let fates = conversation::fates(&entries, "torq-agent-alice", &[40, 41, 42, 99]);
        assert_eq!(fates, [(40, Fate::NotYours), (41, Fate::Waiting), (42, Fate::NotYours), (99, Fate::NotYours)]);
    }

    /// `forget` ends at its error: refused while a task runs, `torq ask --forget` never waits.
    #[test]
    fn forget_ends_at_an_error() {
        use torq_wire::agent::Event;
        let fate = |_| conversation::Fate::Unread;
        let refused = || Event::Error { text: "a task is running: stop it first, or wait".into() };
        assert_eq!(show(refused(), true, &mut false, &fate, &mut Vec::new(), &mut Vec::new()).unwrap(), Some(1));
        assert_eq!(show(refused(), false, &mut false, &fate, &mut Vec::new(), &mut Vec::new()).unwrap(), None, "an ask goes on to its done");
        let mut out = Vec::new();
        assert_eq!(show(Event::Forgotten, true, &mut false, &fate, &mut out, &mut Vec::new()).unwrap(), Some(0));
        assert_eq!(String::from_utf8(out).unwrap(), "The conversation is deleted. Your next question starts a new one.\n");
    }

    /// Ctrl-C asks the agent to stop; a second, or no end within GIVE_UP of the first, leaves.
    #[test]
    fn ctrl_c_stops_then_leaves() {
        let t = std::time::Instant::now();
        let mut i = Interrupt::default();
        assert!(!i.given_up(t + GIVE_UP * 10), "nothing asked yet: never given up");
        assert_eq!(i.key(b'x', t), KeyDoes::Nothing);
        assert_eq!(i.key(0x03, t), KeyDoes::Stop);
        assert!(!i.given_up(t + GIVE_UP / 2));
        assert!(i.given_up(t + GIVE_UP), "no end in time: leave");
        assert_eq!(i.key(b'y', t), KeyDoes::Nothing);
        assert_eq!(i.key(0x03, t), KeyDoes::Leave, "the second Ctrl-C");
    }

    /// The signals that would end `torq ask` with the terminal raw are held and read instead:
    /// a SIGTERM sent to this thread is read from the signalfd as its number, and the thread is
    /// let go of them after (Drop). Sent to this thread alone, so the test's other threads are untouched.
    #[test]
    fn a_signal_is_read_not_taken() {
        use nix::sys::signal::{raise, SigSet, Signal};
        let s = Signals::hold().expect("held");
        raise(Signal::SIGTERM).unwrap();
        assert_eq!(s.next(), Some(15));
        drop(s);
        assert!(!SigSet::thread_get_mask().unwrap().contains(Signal::SIGTERM), "let go again");
    }

    /// A question read from input past 8 KiB is refused, not cut unsaid; one within is taken whole. And the
    /// client names the person's own unit, not a placeholder.
    #[test]
    fn a_long_question_is_refused_and_the_unit_named() {
        let long = format!("{}\n", "x".repeat(9 * 1024));
        assert!(format!("{:#}", question_from(&mut long.as_bytes()).unwrap_err()).contains("longer than 8192 bytes"));
        let fits = format!("{}\n", "y".repeat(8 * 1024 - 1));
        assert_eq!(question_from(&mut "z".repeat(8 * 1024).as_bytes()).unwrap().len(), 8 * 1024, "exactly 8 KiB, no newline: it fits");
        assert_eq!(question_from(&mut fits.as_bytes()).unwrap().len(), 8 * 1024 - 1);
        let c = client::Client::at(std::path::Path::new("/run/torq-agent/alice.sock"), 1000);
        assert_eq!(c.unit(), "torq-agent@alice.service");
    }
}
