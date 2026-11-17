# Security model

## Principles (now)

1. **The config repo is the system, and only the broker writes it.** Every change is a commit, a
   generation and a ledger entry; the ledger is hash-chained and verified.
2. **The AI can never touch the trust layer.** The ledger, the broker's settings, AI permissions
   and boot keys are user-only, changed by a person at a terminal.
3. **Thin.** Few services, and no display server in the base (the desktop is opt-in); what is not installed cannot be attacked.

## Later: owner-rooted boot security

The rest of this document (owner Secure Boot keys, measured boot, TPM2 + PIN disk unlock) is kept
for later, not planned now (ADR-019). It was written for laptops; on TorqOS it applies to any
machine with UEFI and a TPM.

1. **Each machine is its own root of trust.** Keys are generated on the machine at install.
2. **No vendor signatures required.** No shim, no Microsoft keys (default), no project-held
   signing key for boot. The project only signs its package repository/binary cache.
3. **Tamper → locked disk.** If the boot chain changes, the TPM refuses to release the disk key.

## Root, the owner, and an AI the owner runs

The plan that closes this section's gaps is ADR-035, in three levers: *the account* (the broker on
a socket, identity from `SO_PEERCRED`; the agent an account of its own; the owner's password the
human gate), *the door* (strict mode, `torq break-glass`: a root shell only after a ledger entry),
and *the record* (the ledger's head anchored off the machine, ADR-036). What it does not try: to
tell a person from a program at a root shell — nothing can.

- **The account is built (ADR-040, `tests/socket.nix`).** The broker is a socket; who is asking is
  the uid the kernel reports, never a flag. Reading the ledger is anyone's; a person's own folder
  is theirs to change; someone else's folder, or the system, takes an administrator (`wheel`) and
  **their password**, checked by the broker through PAM, with faillock stopping guessing (five
  wrong: ten minutes, counted where only root can reach — and sudo, su and passwd share the count:
  `torq.security.passwordLockout`). Each person's agent account (`torq-agent-<person>`, ADR-051 3) proposes and can do nothing else. A
  program in the owner's shell has their uid but not their password — the human gate. Its limit,
  stated: a program running as the owner could show a fake prompt and read what is typed into it;
  the password is safe from other accounts, and from that program once typed into torqcli (which
  makes itself non-dumpable), not from being asked for by it. `sudo torq-broker` remains the
  recovery door, except in strict mode, where sudo does not run it; its entries say `via: sudo`, which is what root says about itself — `via` and
  `caller` are evidence only for what a non-root account sent over the socket (KNOWN-GAPS 6zf).

- **The `root` account is locked** (no password: nobody logs in as root). **Root's powers** are
  borrowed through `sudo` by the owner, with the owner's own password; `sudo bash` borrows them for a
  whole shell. That is the owner's override, as on any Linux, and TorqOS does not try to forbid it:
  people stay free, the AI is what is limited.
- **What a root shell can do:** go around the broker — edit the config repo or system files, rebuild
  by hand, rewrite the ledger. TorqOS's answer is that going around the door is seen, unless root
  also covers it up:
  drift from the ledger raises `show system alarms` at the next check (every 15 minutes; a change to
  the store itself, daily); `/etc/nixos` refuses a stray rebuild; the
  ledger's hash chain shows an edited entry. Gap: a root writer can rewrite the whole chain
  consistently, and here only the remote's copy disagrees: anchoring the head off
  the machine is the defence, not signing on it (ADR-035; KNOWN-GAPS #2). Root that remains can also
  hide what it did from the checks themselves (rewriting the store's record of its hashes, say): the
  checks are of what root did not cover up.
- **An owner who runs an AI tool in a root shell** has handed it root, and no software can stop
  that. TorqOS makes it **visible** (the same alarms; the ledger records what passed the door —
  as what root says of itself: the broker names its caller from the kernel for what comes through its
  socket, not for a root shell), **recoverable** (every configuration change undoes; `request system repair` returns the
  machine to its last ledgered state, and puts back store paths changed in place that the daily check
  found —
  but what root leaves outside what the configuration names (a file in /etc NixOS does not manage, a
  home's), a rebuild does not touch: only a reinstall from the repository clears it. Data a root process
  deletes comes back from snapshots if root did not delete them too, and a ledger it rewrites is
  caught by the remote's copy), and **unnecessary** (TorqOS's own AI layer: an unprivileged agent that reads
  and proposes, never `sudo`; the owner approves each proposal).
- **Strict mode (optional, off by default):** the owner's `sudo` runs only break-glass and the agent's launcher — no
  root shell to run anything in — with a deliberate, ledgered break-glass for emergencies.
- **Dev mode** (the machine TorqOS is built on) is the labelled exception: passwordless `sudo` and an
  AI coding agent with root. Customer images ship with it off.

## What an agent reaches, and the record of what it asked (ADR-057)

Built on `work/harness-p3` (2026-11-16); what was run is in `HARNESS-AS-A-SYSTEM.md`. Wherever a person has
an agent account:

- **The agent's account reaches one thing on the network: TorqOS's model proxy**, on this machine
  (127.0.0.1, port 7801), and the secrets daemon beside it. A rule of the kernel's packet filter, by the
  account a connection belongs to, refuses every other packet of an agent's: any other address, IPv4 or
  IPv6, any other port of this machine, DNS. The services that would look a name up for anyone (nscd,
  systemd-resolved) are closed to it as well. A person's own account is not touched. Nothing is started as
  an agent while that rule is not loaded: the agent's service and `torq agent run` look at the rule itself,
  not only at its unit, and stopping the fence ends every program running as an agent. Under `torq agent
  run` an agent may open only the kinds of socket the fence covers (unix, IPv4, IPv6, netlink), as in its
  service: not one to the hypervisor (AF_VSOCK), Bluetooth or the like. Neither may use io_uring, which
  makes sockets of any kind past that filter.
- **An agent is never the person at the machine.** polkit refuses every agent account every action, and
  `torq agent run` runs outside the person's login session: an agent cannot power the machine off, change
  its network through NetworkManager, or turn on lingering for itself (which would give it a user manager
  running its own programs outside its sandbox). No agent account lingers.
- **Every request to the model passes through the proxy and is recorded**: whose agent (named by the
  kernel, never by the request), when, which model, how many tokens, what was new in the conversation (the
  question, each tool's result) and what the model answered (its text, each tool it called and with what).
  The proxy passes the request on as it came, the person's sign-in included, and keeps no credential: the
  record, the journal and its error messages hold none. **Its port is held by systemd** from early boot and
  handed to the proxy, never left free between restarts, so no other account can listen there and be sent
  the agents' requests; without that socket, nothing runs as an agent.
- **A site is reached only through the proxy, as HTTPS, and only if it is allowed**: signing in to the
  engine's service; the sites the owner lists (`torq agent network hosts`); every site if the owner turns
  the web on (`torq agent network web`, off by default). Each site asked for is recorded, allowed or
  refused. What passes to an allowed site is encrypted and is not seen.
- **The record is the person's to read and no one else's** (`/var/lib/torq-model-proxy/<person>/`): not
  another person's, not any agent's account, to read or to write. It is kept within a bound, and says where
  something was cut.
- **No agent can change these settings**: they are `torq.*`, which the broker never lets an agent propose.

What this does not do, said plainly (KNOWN-GAPS 6zzx):

- **The record is not the ledger.** Nothing chains its lines: root, and the proxy's own account, can change
  or remove it without a trace. It shows what happened if no one with root altered it.
- **With the web on, or a site listed, the agent can send what it can read to that site**, and the record
  shows only that the site was reached. The fence around what the agent can *read* (its own home, the
  folders shared with it) is what limits the loss.
- **The model's service itself receives whatever the agent sends it.** A request is recorded, not judged.
  And the proxy passes any credential through, so a program run as an agent could ask the service with
  another account's key: the record shows the request, not whose key it carried.
- **The record holds what the agent read**, cut to 8 KiB a text: file contents, command output. It is as
  private as the person's own files, and is kept as such.

## Accounts and passwords (agreed for M2, 2026-11-01; built: INSTALL.md)

- A new image has no user account: only the locked `root`. **First-boot setup** creates the owner's
  account (name and password), asks how to log in remotely — **password, SSH key, or both** — and
  commits it as change #1. It never runs again.
- **Passwords never enter the config repo** (it is readable by everyone and kept forever; a hash
  there can be cracked offline): the configuration declares the account and points at a file only
  the system can read (NixOS `hashedPasswordFile`, outside the repo). A TorqOS command (e.g.
  `request system password`) changes it through the broker; the ledger records *that* it changed,
  when and by whom — never the secret. More users get theirs the same way. Built: the
  configuration tree refuses a person's password in any of NixOS's forms (`users users <name>
  password`, `hashedPassword`, `initialPassword`, `initialHashedPassword`, `hashedPasswordFile`,
  `passwordFile`), in torqcli and at evaluation (modules/settings-fence.nix, rule 3), saying to use
  `request system password <user>`.
- **A backup's key is kept the same way** (ADR-038, amended 2026-11-07): `request system backup key
  <name>` asks for it at the terminal, an administrator's with their password; the broker keeps it
  root's alone in `/var/lib/torq/secrets/backup/<name>`, where the backup's `passwordFile` points, and
  ledgers that it changed. The tree may not point `passwordFile` elsewhere, and `/var/lib/torq/secrets`
  is never in a backup unless its `paths` name it.
- **Nothing listens on the network unless asked for:** no SSH until turned on, and no link-local
  name answering (systemd-resolved's LLMNR and multicast DNS, off: they let anyone on the same
  network answer for a name). tests/image.nix checks the image.
- **SSH is NixOS's own** (`services openssh enable true`; ADR-025), and TorqOS's protections apply
  however it is turned on (modules/services.nix): keys only unless
  `services.openssh.settings.PasswordAuthentication` is set, and then repeated wrong guesses are
  blocked (sshguard); 3 tries a connection; only the owner may log in (`AllowUsers`). These are
  defaults a person can change (tests/ssh.nix checks each). `root` never logs in over SSH: an assertion refuses any
  `PermitRootLogin` but `no`. `show system alarms` notes that SSH accepts passwords. A commit (or
  revert) that changes who can log in over SSH — turning it on, allowing passwords or turning them
  off, removing a key, another owner — shows it (the keys that will be able to log in, or that
  passwords will be accepted), and is refused if SSH is keys only and there are none: with SSH
  already on as much as when it turns on (audit of 2026-11-01). Keys only stays one `set` away.
- **Releases are signed** (ADR-028): each image's checksum, with the TorqOS release key, and the one
  command that checks it (INSTALL.md); and each release's source, a git commit the key signed.
  `request system update` takes only a commit signed (an SSH signature; OpenPGP and X.509 ones are
  refused) with the key the machine was installed with (`/etc/torq/release-signers`), checked by
  torqcli before the update enters the candidate. A2 of the audit of 2026-11-04 (a lock someone else
  wrote, whose content hash points at other source than the signed commit) is covered: before it
  builds a commit that changes `flake.lock`, the broker fetches each changed input again by its `rev`,
  compares the hash, and checks that TorqOS's own releases are signed by the release key
  (`broker/src/inputs.rs`, `tests/release-path.nix`; the audit of 2026-11-08 closed two ways round it,
  KNOWN-GAPS 6zs). Not covered, A3: the machine refusing an older release by itself (CI refuses to
  move `stable` back, and `show | compare` names both versions).

## Threat model

Status (2026-11-11): **built** = in the code and tested; **opt-in** = built and off by default; **not built** = designed only. Disk encryption, Secure Boot, the TPM and sandboxed services are not built.

| Threat | Primary defence | Status | Notes |
|---|---|---|---|
| Stolen machine | LUKS2 full-disk encryption | not built | TPM2 + PIN, recovery key shown once at install. Today a stolen disk is readable |
| Evil maid (boot files tampered while away) | Owner-key Secure Boot + measured boot | not built | Disk won't unseal if PCRs change. Only the boot menu's editor is off (built) |
| Firmware keys reset to factory | Firmware admin password; PCR 7 change → disk stays locked | not built | `show system alarms` warns |
| Malware during a session | read-only /nix/store, few services, sandboxed services | partly: the store is read-only and few services run (built); sandboxed services not built | The broker's units are not sandboxed (KNOWN-GAPS N5) |
| Malware persistence across boots | Signed UKIs only; config rebuilt from repo | config rebuilt from the repository: built. Signed UKIs: not built | Optional impermanence later |
| Prompt injection steering the AI | Broker tiers; every proposal of an agent's waits for a person | built, with limits | The broker cannot see what an agent reads, so it trusts none of it: each proposal is approved by a person, shown as the broker's diff, with what the agent reported reading (ADR-055 phase 2; KNOWN-GAPS 6zzu). A person can still be talked into approving |
| An agent sending what it read to a stranger | The agent's account reaches only the model proxy; sites by the owner's list; each request and each site recorded | built (ADR-057) | Open where the owner lists a site or turns the web on; the model's own service receives what the agent sends it. The record is not tamper-evident (KNOWN-GAPS 6zzx) |
| Compromised signing key on device | Only the db key lives on device (broker-only, TPM-sealed); PK/KEK exported offline then deleted | not built | Needs Secure Boot and the TPM |
| Secure Boot bypass vulns (BlackLotus, LogoFAIL class) | fwupd firmware + dbx updates, applied via broker | not built | |
| A ledger rewritten by root on the machine | Pushing the configuration and ledger to a remote | opt-in | Off by default; without it the hash chain only detects partial edits (KNOWN-GAPS 2) |

## Secure Boot: owner keys, no Microsoft

Key hierarchy in firmware: **PK** (owner) → **KEK** → **db** (allowed) / **dbx** (revoked).

Install flow (later):

1. User puts firmware in **Setup Mode** (installer shows per-model instructions).
2. Generate PK/KEK/db on the machine (`sbctl create-keys`, wrapped by our installer).
3. Enroll keys **plus option-ROM hashes from the TPM event log**
   (`sbctl enroll-keys --tpm-eventlog`) — keeps GPU/TB/NVMe option ROMs working
   without trusting Microsoft.
4. Toggle (default OFF): "Keep Microsoft keys (needed to dual-boot Windows)" → `--microsoft`.
5. Prompt user to set a **firmware admin password**.
6. Export PK + KEK to USB / printed recovery sheet; delete from disk.
7. Reboot with Secure Boot on; Lanzaboote signs every generation's UKI with the db key.
8. Re-enroll LUKS to TPM2 + PIN bound to the measured boot state.

Before any firmware update the broker must check whether option-ROM hashes will change and
re-enroll first (otherwise: black screen risk).

Hardware guidance: integrated-graphics machines are lowest risk for removing Microsoft keys.

## Disk encryption

- LUKS2, `systemd-cryptenroll --tpm2-device=auto --tpm2-with-pin=yes`, PCR policy managed
  via Lanzaboote's measured boot / systemd-pcrlock support.
- Recovery key generated at install, shown once.

## Hardening defaults (initial list, expand with care)

- Kernel lockdown is asked for (integrity), but this kernel has no lockdown module loaded, so it does
  nothing yet (KNOWN-GAPS 6); modules are not signature-checked.
- No display server in the base; the opt-in desktop (COSMIC) adds one.
- Minimal enabled services. TorqOS's own services are not yet sandboxed beyond what systemd does by
  default (`torq-broker-init` only; audit N5).
- `nix.settings.allowed-users` is root, wheel and `torq-check`: the owner can build anything, and root can
  switch by hand (drift, reported, not prevented). Only the broker *ledgers* a switch. An agent's account is
  refused by the daemon, and so is a person who is no administrator (`tests/agent-check.nix`).
- `torq-check` is the account an agent's proposal is evaluated as (ADR-056): the one account besides root and
  the administrators that the Nix daemon answers, because an evaluation needs it. Allowed, never trusted
  (by its name, its group or `*`); no home, no shell, a locked password, no SSH key, no group but its own:
  each set by TorqOS's module and asserted (`tests/agent-assertions.nix`). Nothing runs as it but the
  broker's check, in a transient unit with no capabilities, no network, a read-only system, no home and
  bounded memory, tasks and time. An allowed client can add files to the store and have the daemon build a
  derivation it describes; what the checker is given is an agent's change, which the broker's policy admits
  only as JSON data (package names for its person, values in the subtrees the owner listed). With the
  default list that names no derivation of the agent's choosing. That bound is the policy's, not the
  daemon's, and not a property of data: a listed setting whose type is a package lets the agent name any
  nixpkgs attribute, a listed subtree whose values become scripts puts its text into a derivation, and a
  build the daemon starts during an evaluation (import-from-derivation stays on) is outside the unit's
  limits (KNOWN-GAPS 6zzw). Not chosen: a private store for the checker with no daemon at all (491 MB a
  machine, and a seeding step at every change of the lock).
- The boot menu's entries cannot be edited (systemd-boot's editor off, `modules/security.nix`): a
  kernel command line typed at boot is a root shell. A system-tier commit can turn it on,
  ledgered. Checked on the disk the installer writes (`tests/installer.nix`) until the
  EFI-partition test of ADR-033 checks the menu itself.

