# CLAUDE.md — TorqOS

> **TorqOS** in prose, `torq` in code, commands and paths. Renamed from the placeholder "Kestrel"
> on 2026-10-30; GitHub: `torqosorg/torqos`.

## What this project is

TorqOS is a **thin operating system** built on **NixOS**, where **the configuration is the
device** (ADR-019), with **one base and two faces** (ADR-034): the sysadmin's server as code in
git, and a person's desktop or laptop that an AI can drive safely. Its goals:

1. **One configuration defines the machine.** Like a network device's config: `torqcli` edits one
   tree, every NixOS setting plus plain Nix config files, kept in the machine's config git repo
   (`docs/CONFIG-IS-THE-DEVICE.md`). torq is the small, tightly controlled configuration part of
   the OS; everything else is plain NixOS. With a remote, the machine is a URL (ADR-036).
2. **Every change is controlled, auditable and reversible.** A small privileged **broker** is the
   only writer of the config repo: each change is a git commit that becomes a bootable NixOS
   generation, recorded in a tamper-evident ledger, undoable in one command.
3. **The AI fenced, people free — and next, not later.** The agent is an account the kernel
   vouches for (ADR-035), unprivileged; it only *proposes* configuration changes; people stay free
   (anything in their own home, any change they approve). The desktop is a *configuration*
   (`torqos-desktop`) loaded on the same base, not a product (ADR-034). torqcli is the agent's
   interface first, the administrator's second.

Not goals: laptop-*specific* or touch-first design, reference hardware, a smoothness target;
building a sync service or a cache of our own (ADR-038, ADR-039); preventing root (the agent
cannot get it; the owner can, not silently — ADR-035). Owner-rooted boot security is later
(`docs/ROADMAP.md`).

Read `docs/VISION.md` first, then `docs/ARCHITECTURE.md`. The owner's procedure for steering
sessions — what to build next, where a fresh session validates — is `docs/DRIVING.md`.

## The one rule that shapes everything

**The AI never changes the running system directly.** It edits the system *configuration*
(the local git repo at `/var/lib/torq/config`), and the broker builds the system from it.
That is what makes auditing and reversal automatic. Do not add code paths that bypass this.

## Repository layout

```
flake.nix              entry point: images, VMs, checks, packages, the dev shell
modules/               the distribution, as NixOS modules (torq.* options)
  base.nix             nix settings, users, locale, core packages
  security.nix         hardening (boot chain and TPM: later)
  desktop.nix          a desktop (COSMIC, ADR-045), off in the base; the torqos-desktop configuration's (ADR-034, M4)
  broker.nix           the privileged change broker service + config repo; the account torq-check and the check of an agent's proposal (ADR-056)
  shell.nix            the login shell: zsh + oh-my-zsh and the TorqOS prompt (shell/torq.zsh-theme)
  setup.nix            first-boot setup, at the console of a machine made from the image
  installer.nix        the ISO installer's console: the image onto a chosen disk (ADR-031)
  releases.nix         where a machine takes TorqOS from, signatures checked (ADR-028)
  settings-fence.nix   what the configuration tree may never change
  dev.nix              development mode (ADR-009) — and the assertion that guards a path: source
  secrets.nix          torq-secrets: a program uses a secret and never reads it (ADR-052)
  model-proxy.nix      torq-model-proxy and the fence around the agents' network: what an agent asks its model, recorded; the sites it may reach, `torq.agent.network` (ADR-057)
  agent.nix            each person's agent account torq-agent-<person>, what it may propose, `torq agent run`, the engine (`torq agent engine`) and torq-agent@<person>, the bridge to it, `torq agent journal` (ADR-041, ADR-051 3, ADR-055)
hosts/
  image/               the release image's own system (`.#image`, `.#image-vmdk`)
  installer/           the ISO installer's system and the image it carries (`.#iso`)
  dev-vm/              throwaway VM (`.#nixosConfigurations.dev-vm-x86_64`; an aarch64 one is defined but nothing builds it)
templates/config/      the per-machine config repo the broker manages
                       (`nix flake new -t .#config /var/lib/torq/config`)
broker/                Rust crate: torq-broker (ledger, policy, git, rebuild, rollback)
cli/                   Rust crate: torqcli — the grammar, the candidate, the tree (ADR-025)
agent/                 Rust crate: torq-agent-service — each person's agent on its socket: the bridge to their engine, Claude Code (ADR-055);
                       torq-mcp — TorqOS's tools for a coding tool, an MCP server (ADR-051 7)
proxy/                 Rust crate: torq-model-proxy — the relay to the model, its record, the sites an agent may reach (ADR-057; modules/model-proxy.nix)
wire/                  Rust crate: torq-wire — the socket's types and the ledger's words, shared by the three (ADR-044); the record's words (`activity`, ADR-057)
tests/                 NixOS VM tests
release/               allowed_signers: the keys a machine trusts a release from
.github/workflows/     test (pull requests, by hand, each release), image (by hand, each release), weekly nixpkgs bump, release on a v* tag
branding/              the TorqOS logo, icons, terminal logos and palette (modules/branding.nix)
secrets/               Rust crate: torq-secrets, the secrets daemon (ADR-052; modules/secrets.nix)
scripts/               bump-version (the release) and bump-cosmic
settings/              Rust crate: torq-settings (TorqOS Settings, ADR-045 2; its Agent and Secrets pages), torq-applet (the panel, ADR-049), torq-tunnel(s) (SSH tunnels, ADR-050),
                       torq-agent-applet, torq-agent-ui and torq-crash-watch (the person's agent: panel, window, crashes offered, ADR-051 4),
                       on torqcli's library (torq::agent); cli/ also builds torq-agent-launcher (the launcher's `ask`)
docs/                  design docs and decision log
```

Two repos, on purpose:
- **This repo** = the distribution (modules, broker source, docs). Developed normally, on GitHub.
- **`/var/lib/torq/config`** on each machine = that machine's config, a small flake that imports
  this repo via `torq.lib.mkSystem`. Local by default (an optional remote, ADR-036). Written only by the broker.

## Commands

The Rust toolchain is not installed in the system: it comes from the flake's dev shell, so the
machine stays thin. `nix develop` for a shell, or `nix develop --command …` for one command.

```bash
# The Rust crates
nix develop --command bash -c 'cd broker && cargo test'   # incl. ledger tamper detection
nix develop --command bash -c 'cd cli    && cargo test'   # the grammar, candidate, tree
nix develop --command bash -c 'cd agent  && cargo test'   # the agent's service against a stand-in engine; torq-mcp
nix develop --command bash -c 'cd wire   && cargo test'   # torq-wire: the socket's bytes (ledger fixtures: the broker's)
nix develop --command bash -c 'cd secrets && cargo test'  # torq-secrets
nix develop --command bash -c 'cd proxy  && cargo test'   # torq-model-proxy, against a stand-in for the model's service
# settings/ has no tests of its own; it is type-checked in its own shell: nix develop .#torq-settings

# Nix — x86_64 only: aarch64 is neither built nor tested during development (ADR-030, amended)
nix flake check                                   # eval + tree checks + VM tests
nix build .#checks.x86_64-linux.trust-loop        # one VM test
nix build .#checks.x86_64-linux.image             # boot, setup, a commit, a signed update
nix build .#image                                 # the qcow2 (.#image-vmdk for ESXi)
nix build .#iso                                   # the ISO installer, for real hardware (ADR-031)
nix build .#checks.x86_64-linux.installer         # USB stick → chosen disk → its first boot
nix build .#nixosConfigurations.dev-vm-x86_64.config.system.build.vm
./result/bin/run-torq-dev-vm                      # boot the throwaway VM (dev / torq)

# Against this machine's config repo (dev mode: sudo needs no password)
sudo torq-broker propose --diff change.diff --reason "..." --actor user
sudo torq-broker apply <seq> --approved-by $USER
sudo torq-broker revert <applied-seq> --approved-by $USER
sudo torq-broker log && sudo torq-broker verify && sudo torq-broker status
```

The VM tests need `/dev/kvm` to be quick; the check host has it (ESXi exposes nested
virtualisation). Without it QEMU falls back to TCG and they take minutes.

## Current milestone: after 0.4.0-rc1 (2026-11-14)

Released so far: `v0.1.0` to `v0.3.2` (v0.3.2 on 2026-11-09), and `v0.4.0-rc1` as a pre-release
(2026-11-11): the **desktop** (the COSMIC line) and **secrets** (ADR-052 steps 1 to 3, command line
only). No `rc2` tag exists. Releases, version bumps and upgrade-path hardening are parked by the
owner since 2026-11-12 (there is no install base): polish and the pending features come first.

On `main` and not released: the agents work (ADR-051 as amended by ADR-055: one agent account per person,
`torq-agent-<person>`, its service, applet and window, `torq-mcp`), secrets steps
1 to 5 including the approval flow (ADR-052 step 4: waiting uses, grants, `secret-grant --replace`, the
panel applet's dialog, and the Secrets page in TorqOS Settings), and several polish rounds
(2026-11-12 to 2026-11-14). The old branch `work/agents` and its tag `park/agents-known-good` are
only history now. Nothing may be said to be released that is only on `main`.

The agent's plan (ADR-055, 2026-11-15; `docs/HARNESS-AS-A-SYSTEM.md` is the plan and says what is done): the
machine's agent is the person's coding tool, Claude Code first; TorqOS writes no agent. Phase 1, on `main`, is
the bridge: `torq agent engine claude-code`, the agent's service running that engine as `torq-agent-<person>`,
`claude` typed in a terminal (`torq agent claude`), and a person's standing instructions for their agent (`torq
agent instructions`). Phase 2 (2026-11-16, on `main` since its full check passed at f82d281)
removed TorqOS's own loop: `torq-doctor`, `torq doctor`, `torq agent propose`,
the settings under `torq agent doctor` and `torq agent model` with the local model's unit, the
`torq-resident-<person>` accounts, the broker's resident rule and session trust. Every proposal of an agent's
waits for its person. Do not bring any of it back as a stub: a local model returns with opencode (phase 4).

Being built on `work/harness-p3` (ADR-057, 2026-11-16; not merged, no full check): the first half of phase 3.
The model proxy (`torq-model-proxy`: every request an agent makes of its model passes through it and is
recorded for the person, in `/var/lib/torq-model-proxy/<person>/`), and the agents' accounts' network closed
but for it and the sites the owner allows (`torq agent network hosts`, `torq agent network web`). Not built
yet: showing the record (`show system agent activity`, the page in TorqOS Settings).

Milestones as built (`docs/ROADMAP.md`'s checkboxes are the most current record): M1, M1.5, M2 and M3
are done. M3.5 (the install on real hardware) is partly done: the offline first build, network-first
setup and the laptop install are built; the console UI, the answers file, recipes and the
installer's other ways are not. M4 (the desktop) is largely built. M5 (the agent): v0.3.x released the old
loop's non-GUI half (doctor mode, proposals, session trust), of which doctor mode and session trust are
removed since phase 2; the agent as it is now (ADR-055) is unreleased.

## Engineering principles

- **Privileged code is Rust**, small, and boring. The broker must stay auditable line by line.
- **No `unsafe`, no dead code — the compiler forbids them** (ADR-020): `#![forbid(unsafe_code)]` and
  `#![forbid(dead_code)]` at every crate root. Never add `#[allow(dead_code)]`, `#[allow(unsafe_code)]`
  or an `unsafe` block: use a safe API (rustix, std), delete the dead code, or redesign. A derive
  macro that writes an `allow` (clap's) is not used.
- **The base is CLI only.** The one graphical piece, `settings/` (TorqOS Settings and the panel applets, desktop only), is a client of the broker's socket and has no privilege of its own. The agent runs unprivileged and sandboxed.
- **Everything declarative.** If it isn't in the config repo, it doesn't exist after a rebuild.
- **Test the trust properties, not just features.** Every security or reversibility claim
  should have a NixOS VM test (`tests/`) that would fail if the claim broke.
- **Thin.** What the system does not need is not installed; torq stays small.
- **Prefer existing, maintained components** (NixOS, nixpkgs, the real Linux tools, the person's coding
  tool, MCP). Original work goes into the broker, ledger, torqcli and the agent tools.

## Guardrails for Claude Code working in this repo

- Never weaken a security default to make something work. Add an explicit, documented option
  instead, defaulting to the secure value.
- Never give the agent a code path to: modify its own permissions, disable/edit the ledger,
  touch Secure Boot keys, or run arbitrary root commands. Those are user-only, done by a person at a terminal.
- Record significant design choices in `docs/DECISIONS.md` (short ADR entries).
- Keep `docs/` in sync when behaviour changes.

## Current status (read this first), 2026-11-14

- **What is released:** `v0.1.0` to `v0.3.2`, and `v0.4.0-rc1` as a pre-release (2026-11-11), x86_64
  only (images: qcow2, VMDK; the ISO installer since v0.2.0). No aarch64 downloads exist for 0.3.x,
  and nothing builds or tests aarch64. Releases and version bumps are parked (above).
- **What `main` holds beyond rc1:** the agents work, secrets steps 4 and 5 and several polish rounds
  (above), none of it released. The desktop (in rc1): COSMIC through NixOS's modules, TorqOS Settings,
  a panel applet, an SSH-tunnels applet, approval through a polkit dialog (`modules/desktop.nix`,
  `settings/`, off by default). Setup has a network/Wi-Fi step and a "new machine or from a
  repository" question. The desktop has run on exactly one physical machine, the owner's ASUS
  Zenbook S 16; the agent's applet, window and Agent page, the approval dialog and the Secrets page
  have not been seen on any real desktop (`docs/HANDOFF-agents.md`).
- **Secrets:** `torq-secrets` (`secrets/`, `modules/secrets.nix`). ADR-052 steps 1 to 3 are in rc1.
  Step 4 (an agent's use that asks waits for its person, who approves or denies it at a terminal or in
  the panel applet's dialog; a grant of one secret for minutes, `allow <name> for <minutes>`; `secret-grant --replace`) and
  step 5 (the Secrets page in TorqOS Settings) are on `main`.
- **The agent is experimental.** The `torq-agent-<person>` accounts, `torq agent run`, the agent's service
  (`torq-agent@<person>`, the bridge to the engine, `torq agent engine`), `claude` typed plainly, `torq ask`,
  proposals and `torq-mcp` exist. TorqOS has no agent loop or model of its own (removed in phase 2, above):
  nothing may be written as if `torq doctor`, a local model, a resident agent or session trust existed. Every
  proposal of an agent's waits for its person; the ledger's field for that is still called `tainted`, and a
  person is shown plain words instead. The real Claude Code 2.1.316 was driven through the bridge once, by
  hand, outside its unit, on the check host (2026-11-15); inside the unit only a stand-in has run. The
  default allow-list is the owner's own packages. An agent's account is refused at the Nix daemon, so its
  torqcli evaluates nothing: the broker has each proposal checked first by `torq --check-tree`, as the locked
  account `torq-check` in a unit of its own (ADR-056, KNOWN-GAPS 6zzv; found on the laptop on 2026-11-16,
  where no agent could propose; `tests/agent-check.nix` evaluates a real configuration in a VM; not yet seen
  on the laptop). A person who is no administrator is refused at the daemon too, so cannot commit: open.
  Each use of a secret by an agent asks its
  person, unless the secret's rule says `without-asking` or a grant of that one secret covers it (ADR-052,
  amended 2026-11-16: two words, one kind of grant; a person reads no word about taint or sessions for it).
- **Tests.** Unit tests are `#[test]` functions: torqcli 506, broker 339 (across its test binaries), wire 24,
  secrets 15, agent 59 (+2 ignored) (run 2026-11-16 on `work/harness-p2`, after phase 2 removed the old loop
  and its tests, after secrets were reduced to what they do, and after ADR-056 and its fix round). The
  `settings/` crate has no tests of its own; it is type-checked with `nix develop .#torq-settings`.
  VM tests are `checks` in `flake.nix`, one file each in `tests/`. CI's `test` workflow runs on pull
  requests, by hand and inside each release, **not on pushes to `main`**, and GitHub does not run the
  full `nix flake check` for development: it runs on the check host. The last full `nix flake check`
  passed on 2026-11-16 at f82d281 (phase 2 of `docs/HARNESS-AS-A-SYSTEM.md`, its third run: what the first two failed for
  is in that file); the Secrets page and the agent's UIs
  are type-checked only. On `work/harness-p3` (2026-11-16, no full check): wire 28, torq-model-proxy 37 (+1 ignored: the
  by-hand entry) and the VM test `model-proxy` ran; the real Claude Code 2.1.316 was put through the proxy by hand on
  the check host, outside any unit. No test presses a button in the approval dialog (a VM test has the applet see a waiting use and send its yes). Say what was actually run; do not write that
  something is tested when it only exists.
- **The dev host** is torq5, "the check host" (user `viper`, repo `~/torq`), an x86_64 ESXi VM with KVM, since 2026-11-15: development and every heavy build (`nix flake check`, VM tests, the images) run there. The owner's laptop (hostname `torqlaptop`, user `cwdavis`) is a TorqOS machine, the only one the desktop runs on, and is the try-it-out machine: it follows the `tested` branch and nothing is developed on it. Session worktrees live beside
  `~/torq`; do not edit in another session's.
- **Known limits** are in `docs/KNOWN-GAPS.md` (the desktop and launcher items 6zx and 6zy, and 6zz,
  `torq agent run`'s set-id sweep, are fixed; the agents' own are 6zzk to 6zzo, the approval flow's 6zzs, the bridge's 6zzt,
  the old loop's removal 6zzu, the check of an agent's proposal 6zzv, what that check does not bound 6zzw, the model proxy and the agents' network 6zzx).

## Working here

- `commit check` evaluates the candidate; it does not build (`cli/src/grammar.rs`).
- **Way of working:** small steps; every diff is read by the controlling session before it merges, with
  independent readers for bulk or for a security boundary; one full `nix flake check` on the check host
  for each merged chunk;
  mutation-check new guards (break them, see a test fail); prove claims in the Nix sandbox and a VM test.
- **NixOS's own tools for changing a machine are gone** (ADR-032): the broker builds and switches with
  `nix` itself. Never build `/etc/nixos` or any configuration other than the broker's.
- **A `path:` input copies the working tree, uncommitted changes included**; it does not read git. A
  dirty `~/torq` becomes part of the system the broker ledgers: recorded and reversible, not
  reproducible. `git status` before shipping anything. Dev mode (`torq dev enable true`, ADR-009) and a
  `path:` source are two ledgered commits on top of a normal install (`docs/DEV-SETUP.md`).
- **Ship a distribution change to the running machine through the broker**, so it is ledgered. Commit
  first, then use a private candidate (`-c`), never the interactive one, which every interactive
  session of the user shares:
  ```bash
  torqcli -c "request system update; configure; show | compare; commit comment \"…\""   # asks y/N
  ```
  Read the prompt before answering: it must show only `[edit system software]`, not `[edit system
  software torq]`, which would change where TorqOS comes from. A system commit asks the owner's
  password at the terminal; it is never written into the repo or memory.
- Dev mode is a temporary exception for *building* TorqOS. Do not design the TorqOS agent around
  having root; the agent itself stays unprivileged and broker-only.
