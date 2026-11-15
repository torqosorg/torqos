# AI layer

**Status (2026-11-15): experimental, and being changed (ADR-055, `docs/HARNESS-AS-A-SYSTEM.md`). The machine's agent is now the person's coding tool, Claude Code first (`torq agent engine`), run by each person's agent service as a bridge; TorqOS's own loop (`torq doctor`, `torq agent propose`, the settings under `torq agent doctor`, the second account `torq-resident-<person>`, the broker's "resident agent" rule and session trust) is still here and is removed next (phase 2). Until this page is rewritten then, read what it says of doctor mode, the resident agent and session trust as the old loop's: nothing on a machine earns an untainted session any more, and every proposal from an agent waits for its person. Nothing of this is released; the real Claude Code has been driven through the bridge by hand, outside its unit, once.**

**Status before that (2026-11-14): experimental. M5 (ADR-034) is built in part: the account (ADR-035, ADR-041), doctor mode, proposals and session trust (ADR-042) exist and are in v0.3.x. Doctor is off by default, no model ships, and no real model has ever been tried (the tests use a scripted endpoint). The resident agent, torq-mcp and ADR-051 are on main (steps 1 to 5, and approval of an agent's use of a secret, ADR-052 step 4) and not yet released.**
Everything built so far (the broker as the only writer, actors, tiers, the ledger) was built so
this could be added without a bypass. Each person's agent is an *account* of its own,
`torq-agent-<person>` (ADR-051 3, amending ADR-041 1: `torq.agent.people`, the owner by default),
whose identity the kernel vouches for (`SO_PEERCRED`) — never a flag the caller sets. The broker
knows which agent serves whom from its map in /etc/torq/broker.json: an account is an agent only if
the map names it and its uid is no person's.

## One agent per person (ADR-051 3)

- **Its account:** `torq-agent-<person>`, a static system user in its own group, no password, no
  login, no other group, its home `/var/lib/torq-agent-<person>` (0700, its own). Neither agent reads
  the other's home.
- **What it may propose:** packages only in its own person's folder (`users/<person>/packages.json`),
  never another person's; settings only as `torq.agent.mayPropose.settings` allows (the machine's
  list, the same for every agent). A refusal is ledgered with the agent account that tried.
- **Sessions and trust:** a session is its agent account's: no other agent reports into it or cites
  it. A person's trust (`request system agent trust`) applies only their own agent's proposals,
  judged by the broker's map at the time of the apply, and only one the agent sent itself over the socket.
  Limits (proposals waiting, refusals ledgered,
  sessions started and kept) are counted per agent, so one person's agent cannot crowd out another's.
- **`torq agent run`** runs the caller's own agent (the account of sudo's `SUDO_UID`, which the launcher's rule never
  lets a caller set), never one the caller
  names; a person with none is told so, and so is someone who is not an administrator (the launcher's sudo
  rule is the administrators' group's; torqcli says it in one sentence before sudo would). A restore from a
  backup into any agent's folder is refused. With the agent's service running, `--share` holds it still
  (systemd's freezer) through the walk. A folder shared is inside the person's home, never the home itself,
  and never a hidden folder directly under it or anything inside one (`~/.ssh`, `~/.config`: keys, sign-ins
  and settings live there).
- **`claude`, typed plainly** (`torq agent claude`, which the command on people's PATH runs; `torq agent
  plainCommand`): Claude Code as the person's agent, the folder they are in shared with it as `torq agent run
  --share` would. Where that folder is never shared (the home itself, outside it, a hidden folder of it)
  nothing is shared, one line says so, and Claude Code runs in the agent's own folder: `claude --version`
  works anywhere. The long "Shared with your agent…" line is printed the first time a folder is shared only.
- **Its service** (ADR-051 3; the bridge since ADR-055, 2026-11-15): `torq-agent@<person>.service`, reached only
  by that person (`torq ask "<question>"`, the agent's window and applet). It is there where an engine is chosen
  (`torq agent engine claude-code`; the engine is the program that answers) and runs that engine, Claude Code,
  as one long-lived process for the person's conversation, as `torq-agent-<person>`, in a sandbox at least as
  strict as `torq agent run`'s: its own home and no other, no runtime directory, a /tmp and a /dev/shm of its
  own, no privilege, the rest of the system read-only. It passes questions and answers and decides nothing.
  Stop ends the engine's process and the next question continues the conversation; so does closing the
  window or the terminal that asked. Forget deletes the conversation: the engine is ended, its own record of
  the conversation is deleted from the agent's home (`agent/src/service/engine.rs`, `erase`), and the next
  question starts a new one. The engine signs in by itself, once, in the agent's account (`claude` in a
  terminal, then /login; an administrator's command). What it proposes through TorqOS's tools (`torq-mcp`)
  is a coding tool's at the broker: marked tainted, which to the person means it always waits for them, one
  by one. A proposal's number, as the engine's output names it, is looked up in the ledger before it is shown
  as one to apply, by the window and by `torq ask`.
- **Instructions for your agent** (the owner, 2026-11-15): a text of the person's own that their agent is
  always given. It lives in the person's home, `~/.config/torq/agent-instructions.md` (at most 16 KiB of
  plain text; `torq agent instructions [edit | clear]`, or the Agent page's **Edit…**). The agent cannot
  write it (it has no way into the home, and a hidden folder of the home is never shared), so nothing it
  read can rewrite what it is instructed. The person's own programs read it: torqcli's client sends it with
  every question (`instructions`, a field of the Ask), and the bridge starts the engine with it (Claude Code:
  `--append-system-prompt`, under the line "[Standing instructions from the person this agent works for]").
  When a question brings another text than the running engine was started with, the bridge ends the engine
  and starts it again, continuing the same conversation. Claude Code records its prompt at a conversation's
  first question and keeps to that record when the conversation is continued, so the bridge keeps a mark of
  the text a conversation began with (never the text) and, when the text is another, starts Claude Code with
  `--system-prompt-snapshot off`, which makes it take the new one (both seen by hand with 2.1.316).
  `claude`, typed plainly, gives Claude Code the same text before the person's own words; a conversation
  continued there (`claude --continue`) keeps the instructions it began with. `torq agent run -- claude`
  gives none: it runs what it is told. A file that is too long, not text, or a link is not used, and the
  person is told so where the answer is shown.

## What it is for

Three things a person says to their machine; each is an acceptance test (ROADMAP M5):

| | The agent does | The gate |
|---|---|---|
| "Fix my wifi." | reads logs and hardware (doctor), proposes a system-tier change | the person's one answer; `commit confirmed`, so it reverts itself if the machine does not come back |
| "Build me an application for this." | writes and builds it in the person's home, installs it through the tree (ADR-037, rung 3) | none: user tier; every step a ledger line; `rollback` undoes it |
| "Watch while I try it." | screenshots and input through the compositor's protocols | the person's session; nothing of the broker's |

In order: **doctor mode** (read, explain, propose nothing; built, ADR-042: `torq doctor "<question>"`), **proposals** (built: `torq agent propose "<ask>"`), **session trust** (built: `request system agent trust <minutes>`; "for
the next hour, user-tier changes without asking" — ledgered, revertible, never for system tier).
On a desktop the approval is a dialog with the owner's password, the one thing a program in the
owner's shell does not have; in a terminal it is torqcli's prompt, the same text.

## Components

| Component | Privilege | Language | Role |
|---|---|---|---|
| `torq-doctor` (`agent/`), as the person's `torq-agent-<person>` | unprivileged, sandboxed (`torq agent run`, or the agent service) | Rust | TorqOS's harness: talks to the model (OpenAI-compatible chat API), calls its read-only tools, *proposes* changes (ADR-042); one process per question or task |
| `torq-agent@<person>.service` (`agent/`: `torq-agent-service`), as `torq-agent-<person>` (ADR-055) | unprivileged, the launcher's sandbox and systemd's hardening; socket-activated, its socket the person's alone, SO_PEERCRED checked | Rust | The bridge to the person's engine (ADR-055 3): runs Claude Code as one long-lived process for their conversation, hands it each question (with the person's standing instructions) and streams what it says to `torq ask`, the agent's window and the applet; `stop` ends the engine and everything it started, `forget` also deletes the engine's record of the conversation. It keeps the conversation's name, count and number, and nothing of what was said |
| `torq-mcp` (`agent/`), as `torq-agent-<person>` under `torq agent run` | unprivileged, the launcher's sandbox | Rust | TorqOS's tools for a coding tool (ADR-051 7): an MCP server on standard input and output with the doctor's closed set of tools and `propose_change`; its session reported a coding tool's first, so what it proposes waits for a person |
| `torq-broker` | privileged (root, minimal) | Rust | Only writer of the config repo; tiers, commits, rebuild, rollback, ledger |
| Model runtime | unprivileged (DynamicUser) | llama.cpp (`llama-server`, a unix socket) | Local inference; or a remote endpoint as a setting (ADR-042) |
| torqcli | the person's | Rust | Shows proposals, diffs and the ledger; a person approves at a terminal |

## Permission tiers

| Tier | Examples | Requirement |
|---|---|---|
| `observe` | read logs, hardware info, settings, config repo | automatic, still ledgered |
| `user` | one user's *data* files: today `users/<name>/packages.json` (apps); later themes, dotfiles. Never a `.nix` module (that is `system`, wherever it lives) | approve once per category; /home snapshot first |
| `system` | services, drivers, kernel params, packages | always preview + explicit approval; commit + generation |
| `forbidden` | AI permissions, ledger, Secure Boot keys, firmware password, disabling audit | **never** via agent; user-only, at a terminal |

## Taint tracking

Any agent session that has ingested untrusted content (logs, device names, the ledger, configuration
values; web pages, email, downloaded files) is marked **tainted**. Tainted sessions cannot use
"approve once" shortcuts: every `user` or `system` action requires explicit per-step approval
with the taint source displayed.

Built (ADR-042 6, amended): the taint is the **broker's**. TorqOS's agent starts its session at the
broker and reports each tainting read there before the model sees it; the broker keeps the record in
its own state directory, where the agent cannot write, and nothing clears it. The ledger holds a
session where a proposal cites it: the proposal's entry carries the session and what tainted it.
A proposal names its session and is tainted as the broker's record says;
one that names no session — any other program run as the agent — is tainted; one that names another
agent's session is refused. `request system apply`
says what the session read; session trust never covers a tainted proposal, nor another person's
agent's. The broker holds what it
is told, and cannot see what a program reads (KNOWN-GAPS 6zq): so only the person's resident agent —
a process in `torq-agent@<person>.service`'s own control group, which the broker reads from the peer's pidfd
(ADR-051 step 5) — starts a session untainted and has its proposals judged by it. Every other program of an
agent account (a coding tool, `torq doctor`, anything under `torq agent run`) starts tainted
(`not-the-resident-agent`), and may not cite the resident agent's sessions (KNOWN-GAPS 6zzo). Even the resident agent
starts tainted when its model is one another account here could stand in for — any TCP model on this machine
(`local-tcp-model`), a socket in a place others write — so a local model is trusted only on a root-held socket.

## Ledger format (v0)

Append-only JSON Lines at `/var/lib/torq/state/ledger.jsonl`, owned by the broker.
Entry kinds: `init`, `proposal`, `applied`, `apply_failed`, `reverted`, `revert_failed`,
`interrupted`, `repaired`, `confirmed`, `rolled_back` (an unconfirmed `commit confirmed`), and a
person's `trust_granted`, `trust_revoked`, `trust_expired`, and `declined` (a waiting proposal turned down)
(failures and restores carry `restored: true|false` — see ADR-008).

```json
{"seq":1042,"ts":"2026-10-30T15:20:11Z","actor":"agent","approved_by":"casey",
 "kind":"applied","tier":"user","tainted":false,
 "reason":"User asked for night light at sunset",
 "data":{"proposal":1041,"commit":"8f3a2c1…","files":["desktop.nix"]},
 "prev_hash":"…","hash":"…"}
```

- `hash = sha256(prev_hash || canonical_json(entry_without_hash))`
- Genesis entry has `prev_hash` = 64 zeros.
- `torq-broker verify` recomputes the chain and reports the first broken link.
- Git commits carry a `Ledger-Entry: <seq>` trailer; the ledger stores the commit id → each
  checks the other: `status` does that cross-check (KNOWN-GAPS 2); `verify` alone does not, and cannot detect
  truncation of the newest entries except against a remote copy of the ledger (ADR-036; built, off by default).
- Built, opt-in: the ledger and configuration pushed to a remote after every change (ADR-036). Not built: anchoring in a TPM NV index or an external witness.

## Change lifecycle

1. `propose` — agent or user submits a unified diff against the config repo + a reason.
2. Broker validates: diff applies cleanly, touches only allowed paths for its tier, tier ≤ allowed.
   Paths are what the diff actually changes (applied to a scratch index): both sides of a
   rename, raw names — so renames, deletions and quoted names cannot dodge the forbidden tier.
3. Preview shown; approval collected (torqcli at a terminal, or the desktop's dialog). Who approves is the uid the kernel reports on the broker's socket, never a flag; `--approved-by` is taken only on the root recovery path (`sudo torq-broker`).
4. `apply` — snapshot data (ADR-038), commit, build and switch, ledger append, push and anchor
   (ADR-036). (Signing on the machine is not the defence — ADR-035; the anchor is.)
5. `revert <seq>` (`request system revert` in torqcli) — a new commit that reverses one applied change, rebuild, ledger append. Going back to an earlier whole configuration is `rollback <n>` in torqcli, which loads it into the candidate to commit.

Or, instead of 3–4, `decline` (ADR-051, amended): a person turns a waiting proposal down — `declined` in the
ledger (who; the proposal), a commit at scheduled for it cancelled first, and from then on no path applies it
(apply, commit confirmed, commit at, session trust) and it no longer waits. The person whose own folder it is,
or whose agent proposed it, declines without a password; anyone else, an administrator with theirs; an agent
never.

## Tool inventory

Built (doctor mode, ADR-042, `agent/src/tools.rs`; all read-only): `read_journal`,
`hardware_facts`, `show_configuration`, `show_ledger`, `show_alarms`, `nixos_option_docs`. All but
the option docs return untrusted text (a configuration value too) and
taint the session. In propose mode (ADR-042 6): `propose_change`, structured words of the tree, a
proposal through torqcli as the agent. Each tool has its tier (`tools.rs`, `Tool::tier`): the six read-only
ones `observe`, `propose_change` `propose` (the change's own tier is the broker's). The same set is offered to
coding tools by `torq-mcp` (ADR-051 7), without the screen's (step 6).
Later: `logs.coredump`, `pkg.search`, `power.status`, `secureboot.status`, `files.read_scoped`.
