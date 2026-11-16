//! `torq --check-tree <tree>`: the broker's checker (ADR-056). An agent's account is refused at the Nix
//! daemon, so it cannot evaluate the change it proposes; the broker has this run instead, as the locked
//! account `torq-check` in a unit of its own (modules/broker.nix), on the git tree its own vetting made:
//! the config repo's tree `<tree>` copied out and evaluated once, exactly as a person's `commit` checks
//! theirs (`Machine::evaluate`). It reads the repo and the store and writes one line: no privilege, and
//! nothing a person's torqcli does not do.
//!
//! Standard output is exactly one line of JSON: `{"ok":true,"warnings":[…]}` (the configuration evaluates;
//! its warnings), or `{"ok":false,"why":"…"}` (it does not, and what is wrong). Anything that kept it from
//! evaluating at all (no such tree, no room, no Nix) is no verdict: a word on standard error, exit 2,
//! and the broker records the proposal as not checked.

use crate::session::{Evaluated, Machine};
use crate::{history, pkg, Cli, NotValid, BROKER, REPO};
use anyhow::{bail, Result};
use serde_json::{json, Value};

/// How many warnings are passed on, and how long each (and the reason) may be: the broker keeps no more.
const WARNINGS: usize = 8;
const LENGTH: usize = 300;

/// A git tree's id, in full: 40 lowercase hex digits and nothing else. It is the only thing the broker
/// passes, on a command line (systemd's, which expands `$`): never a ref, a path or an option.
fn tree_id(tree: &str) -> Result<()> {
    if tree.len() != 40 || !tree.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        bail!("a tree is named by its id, 40 lowercase hex digits");
    }
    Ok(())
}

/// What the evaluation says of the tree, as the one line; Err: it could not be evaluated at all.
fn verdict(evaluated: Result<Evaluated>) -> Result<Value> {
    match evaluated {
        // A configuration that does not read the tree's files would take the change and do nothing.
        Ok(e) if !e.reads_tree => Ok(json!({ "ok": false, "why": "this machine's configuration does not read its settings files (its flake.nix must call torq.lib.mkSystem with `configDir = ./.;`)" })),
        Ok(e) => Ok(json!({ "ok": true, "warnings": e.warnings.iter().take(WARNINGS).map(|w| history::one_line(w, LENGTH)).collect::<Vec<_>>() })),
        Err(e) => match e.downcast_ref::<NotValid>() {
            Some(NotValid(why)) => Ok(json!({ "ok": false, "why": history::one_line(&pkg::what_is_wrong(why), LENGTH) })),
            None => Err(e),
        },
    }
}

/// `torq --check-tree <tree>`: the one line, and the exit code.
pub(crate) fn run(tree: &str) -> i32 {
    // The machine's own repo; another only in a development build (as the shell's, shell.rs).
    let repo = if cfg!(debug_assertions) { std::env::var("TORQ_CONFIG_REPO").unwrap_or_else(|_| REPO.into()) } else { REPO.into() };
    let cli = Cli { repo: repo.into(), broker: BROKER.into() };
    match verdict(tree_id(tree).and_then(|()| crate::shell::Real::new(&cli).evaluate(tree, &[]))) {
        Ok(v) => {
            println!("{v}");
            0
        }
        Err(e) => {
            eprintln!("{}", history::one_line(&format!("not checked: {e:#}"), LENGTH));
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evaluated(warnings: &[&str], reads_tree: bool) -> Evaluated {
        Evaluated { warnings: warnings.iter().map(|w| w.to_string()).collect(), reads_tree, ..Default::default() }
    }

    #[test]
    fn a_tree_is_named_by_its_id_and_nothing_else() {
        assert!(tree_id("0123456789abcdef0123456789abcdef01234567").is_ok());
        for bad in ["", "HEAD", "0123456789ABCDEF0123456789abcdef01234567", "0123456789abcdef0123456789abcdef0123456", "0123456789abcdef0123456789abcdef012345678", "--help", "$HOME", "0123456789abcdef0123456789abcdef0123456\n"] {
            assert!(tree_id(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_verdict_is_one_of_two_lines_or_none() {
        assert_eq!(verdict(Ok(evaluated(&[], true))).unwrap().to_string(), r#"{"ok":true,"warnings":[]}"#);
        // Its warnings, each one line, no more than the broker keeps.
        let many: Vec<String> = (0..20).map(|i| format!("warning {i}\nsecond line")).collect();
        let v = verdict(Ok(evaluated(&many.iter().map(String::as_str).collect::<Vec<_>>(), true))).unwrap();
        assert_eq!(v["warnings"].as_array().unwrap().len(), WARNINGS);
        assert!(!v.to_string().contains("\\n"), "{v}");
        // Not valid: what is wrong, without a command for a person to type.
        let v = verdict(Err(NotValid("torq.packages: 'nosuch' (user alice) is not a package in nixpkgs".into()).into())).unwrap();
        assert_eq!(v.to_string(), r#"{"ok":false,"why":"\"nosuch\" isn't a package."}"#);
        assert_eq!(verdict(Ok(evaluated(&[], false))).unwrap()["ok"], false, "a tree nothing reads");
        // Anything else is no verdict at all.
        assert!(verdict(Err(anyhow::anyhow!("running nix eval: No such file or directory"))).is_err());
    }
}
