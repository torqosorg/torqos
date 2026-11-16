# AI layer

**Status (2026-11-16): experimental, and not released.** TorqOS does not write an AI agent (ADR-055,
`docs/HARNESS-AS-A-SYSTEM.md`): the machine's agent is the person's coding tool, Claude Code first (`torq
agent engine`), fenced by TorqOS. TorqOS's own agent loop is gone (phase 2 of ADR-055, 2026-11-16): there is
no `torq-doctor`, no `torq doctor` or `torq agent propose`, no settings under `torq agent doctor` or `torq
agent model`, no local model's service, no second account `torq-resident-<person>`, no "resident agent" rule
at the broker and no session trust. Every proposal from an agent waits for its person, who reads it and
approves it, one by one. The real Claude Code has been driven through the agent's service by hand, outside its
unit, once (2026-11-15); in the unit and in every test a stand-in runs. This page says what exists now; how it
came to be is in `docs/DECISIONS.md` (ADR-041, 042, 051, 055).

Everything built before (the broker as the only writer, actors, tiers, the ledger) was built so that an agent
could be added without a bypass. Each person's agent is an *account* of its own, `torq-agent-<person>`
(ADR-051 3: `torq.agent.people`, the owner by default), whose identity the kernel vouches for
(`SO_PEERCRED`), never a flag the caller sets. The broker knows which agent serves whom from its map in
/etc/torq/broker.json: an account is an agent only if the map names it and its uid is no person's.

## What it is made of

1. **The fence.** The agent runs as its own account, in a sandbox: it sees its own home and the folders the
   person shares, not the person's home. It cannot change the system; it can propose a change.
2. **TorqOS as tools** (`torq-mcp`). The agent reads the configuration, the journal (where the owner allows
   it), the hardware, the ledger and the alarms, and proposes changes, through tools TorqOS gives it.
3. **The bridge** (`torq-agent@<person>.service`). It hands a question from the desktop or `torq ask` to the
   engine and brings the answer back as it is written. It decides nothing.
4. **The model proxy** (`torq-model-proxy`, ADR-057; built on `work/harness-p3`, the record not yet shown
   anywhere). Every request the engine makes to its model goes through a TorqOS service on the machine that
   passes it on unchanged and records it for the person: when, which model, how much, what was asked and
   what the agent had read. It is also the only thing the agent's account reaches on the network: a site
   is reached through it, if the owner allows that site (`torq agent network hosts`, `torq agent network
   web`), and each site asked for is recorded.

## One agent per person (ADR-051 3)

- **Its account:** `torq-agent-<person>`, a static system user in its own group, no password, no login, no
  other group, its home `/var/lib/torq-agent-<person>` (0700, its own). No agent reads another's home. There
  is one account for each person's agent, and no other.
- **What it may propose:** packages only in its own person's folder (`users/<person>/packages.json`), never
  another person's; settings only as `torq.agent.mayPropose.settings` allows (the machine's list, the same
  for every agent). `torq.*` is never the agent's: its own engine, tools and journal access are there. A
  refusal is ledgered with the agent account that tried.
- **Limits** (proposals waiting, refusals ledgered, sessions started and kept) are counted per agent, so one
  person's agent cannot crowd out another's.
- **`torq agent run`** runs the caller's own agent (the account of sudo's `SUDO_UID`, which the launcher's
  rule never lets a caller set), never one the caller names; a person with none is told so, and so is
  someone who is not an administrator (the launcher's sudo rule is the administrators' group's; torqcli says
  it in one sentence before sudo would). A restore from a backup into any agent's folder is refused. With the
  agent's service running, `--share` holds it still (systemd's freezer) through the walk. A folder shared is
  inside the person's home, never the home itself, and never a hidden folder directly under it or anything
  inside one (`~/.ssh`, `~/.config`: keys, sign-ins and settings live there).
- **`claude`, typed plainly** (`torq agent claude`, which the command on people's PATH runs; `torq agent
  plainCommand`): Claude Code as the person's agent, the folder they are in shared with it as `torq agent run
  --share` would. Where that folder is never shared (the home itself, outside it, a hidden folder of it)
  nothing is shared, one line says so, and Claude Code runs in the agent's own folder: `claude --version`
  works anywhere. The long "Shared with your agent…" line is printed the first time a folder is shared only.
- **Its service** (ADR-051 3, ADR-055): `torq-agent@<person>.service`, reached only by that person (`torq ask
  "<question>"`, the agent's window and applet). It is there where an engine is chosen (`torq agent engine
  claude-code`; the engine is the program that answers) and runs that engine, Claude Code, as one long-lived
  process for the person's conversation, as `torq-agent-<person>`, in a sandbox at least as strict as `torq
  agent run`'s: its own home and no other, no runtime directory, a /tmp and a /dev/shm of its own, no
  privilege, the rest of the system read-only. Stop ends the engine's process and the next question
  continues the conversation; so does closing the window or the terminal that asked. Forget deletes the
  conversation: the engine is ended, its own record of the conversation is deleted from the agent's home
  (`agent/src/service/engine.rs`, `erase`), and the next question starts a new one. The engine signs in by
  itself, once, in the agent's account (`claude` in a terminal, then /login; an administrator's command).
  A proposal's number, as the engine's output names it, is looked up in the ledger before it is shown as one
  to apply, by the window and by `torq ask`.
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
| "Fix my wifi." | reads logs and hardware (TorqOS's tools), proposes a system-tier change | the person's one answer; `commit confirmed`, so it reverts itself if the machine does not come back |
| "Build me an application for this." | writes and builds it in the folder the person shared, proposes installing it through the tree (ADR-037, rung 3) | the person's answer to the proposal; a ledger line; `rollback` undoes it |
| "Watch while I try it." | screenshots and input through the compositor's protocols (not built) | the person's session; nothing of the broker's |

On a desktop the approval of a system change is a dialog with the owner's password, the one thing a program
in the owner's shell does not have; in a terminal it is torqcli's prompt, the same text.

## Components

| Component | Privilege | Language | Role |
|---|---|---|---|
| The engine: Claude Code, as the person's `torq-agent-<person>` | unprivileged, sandboxed (the agent's service, or `torq agent run`) | not TorqOS's | The agent itself. The person installs it (`torq agent tools`) and signs in; TorqOS ships none |
| `torq-agent@<person>.service` (`agent/`: `torq-agent-service`), as `torq-agent-<person>` (ADR-055) | unprivileged, the launcher's sandbox and systemd's hardening; socket-activated, its socket the person's alone, SO_PEERCRED checked | Rust | The bridge to the person's engine: runs Claude Code as one long-lived process for their conversation, hands it each question (with the person's standing instructions) and streams what it says to `torq ask`, the agent's window and the applet; `stop` ends the engine and everything it started, `forget` also deletes the engine's record of the conversation. It keeps the conversation's name, count and number, and nothing of what was said |
| `torq-mcp` (`agent/`), as `torq-agent-<person>` | unprivileged, the same sandbox | Rust | TorqOS's tools for the engine (ADR-051 7): an MCP server on standard input and output with a closed set of tools, six that read and `propose_change`; linked into Claude Code's settings in the agent's home at each start |
| `torq-broker` | privileged (root, minimal) | Rust | Only writer of the config repo; tiers, commits, rebuild, rollback, ledger |
| The checker: `torq --check-tree`, as `torq-check` (ADR-056) | unprivileged, a locked account of its own, in a transient unit with no network, no home and bounded memory and time | Rust (torqcli's own check) | Evaluates the tree an agent's proposal leaves, before the broker records it: valid, not valid (refused), or not checked (recorded, and said) |
| `torq-model-proxy` (`proxy/`), as `torq-model-proxy` (ADR-057) | unprivileged, a locked account of its own; a unit that can write the record and nothing else; listens on 127.0.0.1 only | Rust (hyper, rustls) | Passes each request of an agent's to the model's service as it came and records it for the person; allows or refuses each site an agent asks for, by the owner's settings; names its caller from the kernel |
| torqcli | the person's | Rust | Shows proposals, diffs and the ledger; a person approves at a terminal |

There is no model runtime on the machine: the engine asks its maker's service, through the model proxy. A local model returns
with opencode (phase 4 of `docs/HARNESS-AS-A-SYSTEM.md`).

## Permission tiers

| Tier | Examples | Requirement |
|---|---|---|
| `observe` | read logs, hardware info, settings, config repo | automatic, still ledgered |
| `user` | one user's *data* files: today `users/<name>/packages.json` (apps); later themes, dotfiles. Never a `.nix` module (that is `system`, wherever it lives) | the person's approval, for each proposal; /home snapshot first |
| `system` | services, drivers, kernel params, packages | always preview + explicit approval; commit + generation |
| `forbidden` | AI permissions, ledger, Secure Boot keys, firmware password, disabling audit | **never** via agent; user-only, at a terminal |

## An agent's proposals always wait

The broker cannot see what a program run as an agent reads: the web, a file it fetched, what its model was
told. So it treats everything an agent proposes as possibly steered by text someone else wrote:

- Every proposal of an agent's is marked in the ledger (the entry's `tainted` field, the ledger's own word
  from v0, kept because the ledger's bytes never change) and waits for a person. No apply happens without one:
  there is no setting, grant or kind of session that applies an agent's proposal by itself. Session trust
  ("for the next hour, user-tier changes without asking", ADR-042 7) existed until phase 2 of ADR-055 and
  only TorqOS's own loop could earn it; it was removed with the loop.
- An apply of such a proposal must say that the person looked (`--confirm-tainted` at the broker; torqcli
  asks in words, TorqOS Settings shows the proposal's page). A `commit at` refuses it: it is applied by a
  person who is there.
- A person reads where it comes from in plain words, at torqcli's apply prompt and on the Proposals page:
  "Your agent proposed this" (to anyone else, "<person>'s agent proposed this"), and which of TorqOS's tools
  it reported reading with ("the system's log, the record of changes"), with
  the advice to judge the change itself and not the reason given for it.
- **Sessions.** A program of the agent's (torq-mcp) starts a session at the broker; the broker keeps its
  record in its own state directory, where the agent cannot write. Every session is marked from its start
  (`agent-program`: one whose reading the broker cannot see), and what the program reports reading is added
  (a tool's name), for good. A proposal that names its session carries that list in its ledger entry. One
  agent never cites or reports into another's session. The same sessions count and bound an agent's uses of
  secrets (ADR-052 4).
- **What this means for secrets** (ADR-052 4, amended 2026-11-16): the broker cannot see what an agent has
  read, so each use of a secret by an agent asks its person, each time. Two things let a use through unasked,
  each for one named secret and each given with the person's password: the secret's own rule saying
  `without-asking` (a person's own secret only), or a grant of that secret for some minutes (`request system
  secret allow <name> for <minutes>`). There is no other word for when a person is asked and no other kind of
  grant; a rule or a ledger from before is read as what it did then (`first-use` and `never` as `each-use`,
  `never-even-tainted` as `without-asking`; the grants of the kinds that went as none).

A person's own proposal is marked the same way only if the one who made it says so (`torq-broker propose
--tainted`, the root path).

## Ledger format (v0)

Append-only JSON Lines at `/var/lib/torq/state/ledger.jsonl`, owned by the broker.
Entry kinds: `init`, `proposal`, `applied`, `apply_failed`, `reverted`, `revert_failed`,
`interrupted`, `repaired`, `confirmed`, `rolled_back` (an unconfirmed `commit confirmed`), `declined` (a
waiting proposal turned down), and a person's `trust_granted`, `trust_revoked` and `trust_expired`, which
today are leave for their agent to use one secret without asking (`data.covers`: `secret <name>`; a ledger
from before 2026-11-16 may also say `all-secrets`, and `proposals` from before phase 2 of ADR-055: neither is
honoured). Failures and restores carry `restored: true|false` (ADR-008).

```json
{"seq":1042,"ts":"2026-10-30T15:20:11Z","actor":"agent","approved_by":"casey",
 "kind":"applied","tier":"user","tainted":true,
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
- **Words from before phase 2.** A ledger written before 2026-11-16 can hold what no broker writes any more:
  a `trust_granted` that covers proposals (`covers: proposals`, or no `covers` at all), its `trust_revoked`
  and `trust_expired`, an `applied` entry with `data.trust` (the grant it was applied under), `data.resident`,
  a caller named `torq-resident-<person>`, and sessions' `tainted_by` naming `not-the-resident-agent`,
  `coding-tool`, `local-tcp-model`, `model-socket-open` or `model-unknown`. An entry's kind and data are
  free text to `verify`, which checks the chain: such a ledger verifies as it always did
  (`wire/tests/fixtures/before-phase-2.jsonl`, written by the old broker, is held to that). Such a grant is
  honoured by nothing; torqcli shows those entries with the reasons they were written with, says of such a grant
  that it no longer holds, treats an agent's proposal left unmarked as an agent's, and reads the old
  session words as "your agent proposed this".

## Change lifecycle

1. `propose` — agent or user submits a unified diff against the config repo + a reason.
2. Broker validates: diff applies cleanly, touches only allowed paths for its tier, tier ≤ allowed.
   Paths are what the diff actually changes (applied to a scratch index): both sides of a
   rename, raw names — so renames, deletions and quoted names cannot dodge the forbidden tier.
   An agent's proposal that passed is then checked (ADR-056): an agent's account is refused at the Nix
   daemon, so its torqcli evaluates nothing; the broker writes the scratch index as a git tree and has
   `torq --check-tree <tree>` evaluate it as the account `torq-check`, the broker's lock let go meanwhile
   and everything vetted again after. Valid: recorded, with the tree and the warnings. Not valid (and the
   configuration as it is valid, asked again each time: nothing is kept between checks): refused, ledgered,
   one plain line back. Could not be checked (a failure that is not the change's among them): recorded,
   and the agent and the person are both told. A person's own change is checked by their own torqcli, as
   before.
3. Preview shown; approval collected (torqcli at a terminal, or the desktop's dialog). Who approves is the uid the kernel reports on the broker's socket, never a flag; `--approved-by` is taken only on the root recovery path (`sudo torq-broker`).
4. `apply` — snapshot data (ADR-038), commit, build and switch, ledger append, push and anchor
   (ADR-036). (Signing on the machine is not the defence — ADR-035; the anchor is.)
5. `revert <seq>` (`request system revert` in torqcli) — a new commit that reverses one applied change, rebuild, ledger append. Going back to an earlier whole configuration is `rollback <n>` in torqcli, which loads it into the candidate to commit.

Or, instead of 3–4, `decline` (ADR-051, amended): a person turns a waiting proposal down — `declined` in the
ledger (who; the proposal), a commit at scheduled for it cancelled first, and from then on no path applies it
(apply, commit confirmed, commit at) and it no longer waits. The person whose own folder it is,
or whose agent proposed it, declines without a password; anyone else, an administrator with theirs; an agent
never.

## Tool inventory

Built (`agent/src/tools.rs`, offered by `torq-mcp`, ADR-051 7): six that only read, `read_journal`,
`hardware_facts`, `show_configuration`, `show_ledger`, `show_alarms`, `nixos_option_docs`; and
`propose_change`, structured words of the tree, a proposal through torqcli as the agent. All but the option
docs return untrusted text (a configuration value too): what they return is cleaned, cut, marked as data and
not instructions, and the read is reported to the broker before a word of it is returned. Each tool has its
tier (`tools.rs`, `Tool::tier`): the six read-only ones `observe`, `propose_change` `propose` (the change's own
tier is the broker's). `read_journal` is offered only where the owner lets the agents read the system journal
(`torq agent journal`, off by default: on, every agent account joins `systemd-journal`, so everything run as
an agent reads every service's and every account's log). Nothing for the screen (ADR-051 6: not built).
Later: `logs.coredump`, `pkg.search`, `power.status`, `secureboot.status`, `files.read_scoped`.
