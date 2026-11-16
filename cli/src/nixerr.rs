//! What a failed Nix evaluation says, as one line a person can act on. Nix ends its output with the
//! error that stopped it: `error: <message>` — or a bare `error:` with the message on the lines
//! after it (failed assertions, a value of the wrong type) — above which come `… while evaluating`
//! traces. The message is that last error's text, without the traces, source excerpts or hints.

/// The message of the last error in `stderr` (Nix's), its lines joined; None if there is none.
pub fn message(stderr: &str) -> Option<String> {
    let lines: Vec<&str> = stderr.lines().collect();
    let start = lines.iter().rposition(|l| l.trim_start().starts_with("error:"))?;
    let mut parts: Vec<String> = Vec::new();
    let first = lines[start].trim_start().trim_start_matches("error:").trim();
    if !first.is_empty() {
        parts.push(first.to_string());
    }
    for l in &lines[start + 1..] {
        let t = l.trim();
        // Not the message: traces, source excerpts (`at …`, `  12|  …`), Nix's own hints.
        if t.is_empty() || t.starts_with('…') || t.starts_with("at ") || t.contains('|') && t.split('|').next().is_some_and(|n| n.trim().chars().all(|c| c.is_ascii_digit())) || t.starts_with("(use '--show-trace'") {
            continue;
        }
        parts.push(t.to_string());
    }
    let joined = parts.join(" ");
    let joined = joined.replace("Failed assertions: - ", "").replace("Failed assertions:", "");
    (!joined.trim().is_empty()).then(|| joined.trim().to_string())
}

/// The configuration's warnings (NixOS's `warnings`) in `stderr`, each one line: Nix writes each as
/// `evaluation warning: <text>` (older Nix: `trace: warning: <text>`), its later lines indented under
/// the first. Nix's own notes (`warning: Git tree … is dirty`) are not the configuration's.
pub fn warnings(stderr: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut open = false;
    for l in stderr.lines() {
        if let Some(w) = l.strip_prefix("evaluation warning: ").or_else(|| l.strip_prefix("trace: warning: ")) {
            found.push(w.trim().to_string());
            open = true;
        } else if let Some(last) = found.last_mut().filter(|_| open && l.starts_with(' ') && !l.trim().is_empty()) {
            last.push(' ');
            last.push_str(l.trim());
        } else {
            open = false;
        }
    }
    found
}

/// Nix was stopped by a signal (the kernel, out of memory; its unit's time or memory limit).
pub const STOPPED: &str = "Nix was stopped before it ended";
pub const NO_DAEMON: &str = "Nix could not reach its daemon or its store";
pub const NO_SPACE: &str = "no space left";
pub const NO_MEMORY: &str = "out of memory";

/// What a failed `nix eval` failed for, where that is plainly not the configuration it evaluated; None: it
/// may be the configuration's, and is taken as that. `code` is Nix's exit code (None: a signal ended it),
/// `stderr` what it wrote.
///
/// Nix exits 1 for all of these, as for a configuration that is wrong, so beyond the signal only its
/// words tell them apart. As little of them is read as will do: the error that stopped Nix (`message`),
/// and in it only how Nix itself begins when it cannot reach its daemon or its store, and how the
/// system's own words for a failed call end it (`<what Nix did>: <the system's words>`): never a word
/// anywhere inside the message, where a configuration's own text can be quoted. What this misses is
/// taken for the configuration's; the broker asks about the configuration as it is before it believes
/// that (ADR-056).
pub fn outside(code: Option<i32>, stderr: &str) -> Option<&'static str> {
    if code.is_none() {
        return Some(STOPPED);
    }
    let m = message(stderr)?;
    // Nix's own first words: the client's socket to the daemon, and the store it opens through it.
    if ["cannot connect to socket at ", "cannot open connection to remote store "].iter().any(|w| m.starts_with(w)) {
        return Some(NO_DAEMON);
    }
    // The system's words (strerror, in the C locale the checker runs in), which end Nix's error for a failed call.
    let ends = |words: &[&str]| words.iter().any(|w| m.ends_with(w));
    if ends(&["Connection refused", "Connection reset by peer", "Broken pipe"]) {
        Some(NO_DAEMON)
    } else if ends(&["No space left on device", "Disk quota exceeded"]) {
        Some(NO_SPACE)
    } else if ends(&["Cannot allocate memory"]) || m == "out of memory" {
        Some(NO_MEMORY)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_configurations_warnings() {
        let stderr = "warning: Git tree '/x' is dirty\nevaluation warning: first warning\nevaluation warning: second\n                    with two lines\ntrace: warning: an older Nix's\nwarning: not the configuration's\n    nor this\n";
        assert_eq!(warnings(stderr), ["first warning", "second with two lines", "an older Nix's"]);
        assert!(warnings("").is_empty());
    }

    #[test]
    fn the_message_of_the_last_error() {
        // A failed assertion: the message is on the lines after a bare `error:`.
        let assertion = "error:\n       … while calling the 'head' builtin\n         at /nix/store/x-source/lib/attrsets.nix:1696:13:\n         1695|           if length values == 1 || pred here (elemAt values 1) (head values) then\n             |             ^\n       error:\n       Failed assertions:\n       - The ‘fileSystems’ option does not specify your root file system.\n";
        assert_eq!(message(assertion).as_deref(), Some("The ‘fileSystems’ option does not specify your root file system."));
        // A message on the `error:` line itself, and Nix's hint after it.
        let plain = "error: torq.packages: 'htpo' (user viper) is not a package in nixpkgs\n(use '--show-trace' to show detailed location information)\n";
        assert_eq!(message(plain).as_deref(), Some("torq.packages: 'htpo' (user viper) is not a package in nixpkgs"));
        // A wrong type: the definition values are part of the message.
        let typed = "error: A definition for option `users.users.alice.shell' is not of type `null or package or shell package'. Definition values:\n       - In `/nix/store/y-source/nixos/users.json': \"zsh\"\n";
        let m = message(typed).unwrap();
        assert!(m.starts_with("A definition for option `users.users.alice.shell' is not of type") && m.ends_with("\"zsh\""), "{m}");
        assert_eq!(message("warning: Git tree is dirty\n"), None, "no error");
        assert_eq!(message("error:\n"), None, "an error with no words");
    }

    #[test]
    fn what_is_plainly_not_the_configurations() {
        // As Nix 2.34 writes them: under an evaluation's trace, or alone.
        let no_daemon = "error:\n       … while calling the 'toFile' builtin\n         at «string»:1:1:\n            1| builtins.toFile \"x\" \"y\"\n             | ^\n\n       error: cannot connect to socket at '/nix/var/nix/daemon-socket/socket': No such file or directory\n";
        assert_eq!(outside(Some(1), no_daemon), Some(NO_DAEMON));
        assert_eq!(outside(Some(1), "error: cannot open connection to remote store 'daemon': error: reading from file: Connection reset by peer\n"), Some(NO_DAEMON));
        assert_eq!(outside(Some(1), "error: writing to file: Broken pipe\n"), Some(NO_DAEMON));
        assert_eq!(outside(Some(1), "error: writing to file: No space left on device\n"), Some(NO_SPACE));
        assert_eq!(outside(Some(1), "error: creating directory '/tmp/nix-1': Disk quota exceeded\n"), Some(NO_SPACE));
        assert_eq!(outside(Some(1), "error: unable to fork: Cannot allocate memory\n"), Some(NO_MEMORY));
        assert_eq!(outside(Some(1), "error: out of memory\n"), Some(NO_MEMORY));
        // A signal, whatever was written.
        assert_eq!(outside(None, ""), Some(STOPPED));
        assert_eq!(outside(None, "error: torq.packages: 'x' (user alice) is not a package in nixpkgs\n"), Some(STOPPED));
        // The configuration's own: a package that is none, a failed assertion, a wrong type; and nothing at all.
        for theirs in [
            "error: torq.packages: 'htpo' (user viper) is not a package in nixpkgs\n",
            "error:\n       Failed assertions:\n       - The ‘fileSystems’ option does not specify your root file system.\n",
            "error: A definition for option `users.users.alice.shell' is not of type `null or package or shell package'. Definition values:\n       - In `/nix/store/y-source/nixos/users.json': \"zsh\"\n",
            "",
            "warning: Git tree is dirty\n",
        ] {
            assert_eq!(outside(Some(1), theirs), None, "{theirs}");
        }
        // The words inside a message, where a configuration's own text is quoted, are not Nix's: a value or a
        // name that says them makes no failure another's.
        for quoted in [
            "error: torq.packages: 'No space left on device' (user alice) is not a package in nixpkgs\n",
            "error: A definition for option `services.x.note' is not of type `path'. Definition values:\n       - In `/nix/store/y-source/nixos/services.json': \"cannot connect to socket at /x: Connection refused\"\n",
            "error: The option `out of memory' does not exist.\n",
            "error: attribute 'cannot open connection to remote store ' missing\n",
        ] {
            assert_eq!(outside(Some(1), quoted), None, "{quoted}");
        }
    }
}
