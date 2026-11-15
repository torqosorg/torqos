//! `torq-desktop-defaults`: TorqOS's applets in a person's own COSMIC panel, once (the library's
//! `torq::cosmic_defaults`). Run by the person's own session at login; nothing privileged.

// No unsafe code and no dead code, ever (ADR-020).
#![forbid(unsafe_code)]
#![cfg_attr(not(test), forbid(dead_code))]
#![cfg_attr(test, deny(dead_code))]

fn main() {
    let said = torq::cosmic_defaults::Places::of_home().and_then(|p| torq::cosmic_defaults::run(&p));
    match said {
        Ok(lines) => lines.iter().for_each(|l| println!("torq-desktop-defaults: {l}")),
        Err(e) => {
            // Not a failure of the login: the file is left as it was, and the next login tries again.
            eprintln!("torq-desktop-defaults: left as it was: {e:#}");
        }
    }
}
