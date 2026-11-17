//! torq-agent-scope [--families <filter>] -- <program> [<argument>…]: what runs as an agent, kept to its own
//! (ADR-057, the security reader, 2026-11-17). Run as the agent, just before the program it starts, in both ways
//! anything runs as one (the agent's service, modules/agent.nix; `torq agent run`, modules/agent-run.sh); then it
//! is that program.
//!
//! An agent shares the machine's network namespace (ADR-057, "considered and not chosen"), so every unix socket
//! with an abstract name (`@/tmp/.X11-unix/X0`, Xwayland's on a desktop) was in its reach: the packet filter does not
//! see them, and no file's permissions guard them. The kernel's Landlock scopes close them: a program here may
//! connect to an abstract socket made by itself or what it started, and to no other; and may send a signal only to
//! itself and what it started (`LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET`, `LANDLOCK_SCOPE_SIGNAL`: Linux 6.12, Landlock's
//! sixth version). Where the kernel has not got them, nothing runs: it says so, plainly.
//!
//! `--families <filter>`: a seccomp filter to load first, as written by modules/agent-socket-families.c (the
//! socket families an agent may open, `socketpair` of no family but unix, no io_uring). The agent's service gives
//! it, where systemd's own filter judges `socket` but not `socketpair`; under `torq agent run` bubblewrap loads the
//! same file before this runs.

#![forbid(unsafe_code)]
// No unsafe code and no dead code, ever (ADR-020); in a test build `deny`, as the service's.
#![cfg_attr(not(test), forbid(dead_code))]
#![cfg_attr(test, deny(dead_code))]

use landlock::{CompatLevel, Compatible, Ruleset, RulesetAttr, RulesetStatus, Scope};
use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};

const USAGE: &str = "torq-agent-scope [--families <filter>] -- <program> [<argument>…]: started by TorqOS before what runs as an agent";

fn main() -> ExitCode {
    let mut args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let mut families = None;
    if args.first().is_some_and(|a| a == "--families") && args.len() > 1 {
        families = Some(args[1].clone());
        args.drain(..2);
    }
    if args.first().is_none_or(|a| a != "--") || args.len() < 2 {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let program = args[1].clone();
    let rest = &args[2..];
    if let Some(path) = families {
        if let Err(e) = load_families(std::path::Path::new(&path)) {
            eprintln!("TorqOS: what runs as an agent could not be kept to the socket families it may use ({e}): nothing was run.");
            return ExitCode::from(1);
        }
    }
    if let Err(e) = keep_to_itself() {
        eprintln!("TorqOS: {e}: nothing was run as your agent.");
        return ExitCode::from(1);
    }
    let e = Command::new(&program).args(rest).exec();
    eprintln!("TorqOS: {} could not be started: {e}", program.to_string_lossy());
    ExitCode::from(127)
}

/// The kernel's Landlock scopes for this process and all it starts: no abstract unix socket but its own, no
/// signal but to its own. Hard requirements: on a kernel without them, an error in plain words.
fn keep_to_itself() -> Result<(), String> {
    let refused = |e: &dyn std::fmt::Display| {
        format!("this kernel cannot keep a program run as an agent from the other programs of the machine (Landlock's scopes, Linux 6.12 or later, are needed: {e})")
    };
    let status = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .scope(Scope::AbstractUnixSocket | Scope::Signal)
        .map_err(|e| refused(&e))?
        .create()
        .map_err(|e| refused(&e))?
        .restrict_self()
        .map_err(|e| refused(&e))?;
    if status.ruleset != RulesetStatus::FullyEnforced {
        return Err(refused(&"they are not in force"));
    }
    Ok(())
}

/// The seccomp filter in `path` (raw BPF, as libseccomp exports it: eight bytes an instruction), loaded for this
/// process and all it starts.
fn load_families(path: &std::path::Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.is_empty() || bytes.len() % 8 != 0 {
        return Err(format!("{} is not a filter", path.display()));
    }
    let program: Vec<seccompiler::sock_filter> = bytes
        .chunks_exact(8)
        .map(|i| seccompiler::sock_filter { code: u16::from_ne_bytes([i[0], i[1]]), jt: i[2], jf: i[3], k: u32::from_ne_bytes([i[4], i[5], i[6], i[7]]) })
        .collect();
    seccompiler::apply_filter(&program).map_err(|e| e.to_string())
}
