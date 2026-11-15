# TorqOS harness as a system

**The plan, and where it stands.** Decided by the owner on 2026-11-15 (ADR-055). This file is the one
place that says what is being built, in which order, and what is done. A session that picks the work up
reads this first and does the next unchecked step; it does not start something that is not here. A
change of plan is written here first, with its date and the owner's word.

## What it is

TorqOS does not write an AI agent. The person chooses one that already exists (Claude Code first, then
opencode) and it becomes **the machine's agent**: every place in TorqOS that offers AI (the agent's
window and panel applet, the launcher's `ask`, Super+Shift+A, a crash's offer, "Ask the agent" in TorqOS
Settings, `torq ask` at a terminal) reaches that one agent.

TorqOS is the system around it. Four parts:

1. **The fence** (exists). The agent runs as an account of its own, `torq-agent-<person>`, in a sandbox:
   it sees its own home and the folders the person shares, not the person's home. It cannot change the
   system; it can propose a change, which the person approves, and which is ledgered and reversible.
2. **TorqOS as tools** (exists: `torq-mcp`). The agent reads the configuration, the journal, the
   hardware and the ledger, and proposes changes, through tools TorqOS gives it.
3. **The bridge** (new). A small service that hands a question from the desktop to the engine and
   brings the answer back as it is written. It decides nothing.
4. **The model proxy** (new). Every request the engine makes to its model goes through a TorqOS
   service on the machine, which records it: who, when, which model, how much, what was asked and what
   the agent had read. This is the audit of AI on the machine, whatever the engine is.

What is removed: TorqOS's own agent loop (`torq-doctor`, its prompts, its model client), the one-shot
`torq doctor`, the model settings under `torq agent doctor`, and the second account
`torq-resident-<person>`. Breaking changes are accepted: TorqOS has no users yet.

## Why (the owner, 2026-11-15)

"I wanted Claude Code to be the agent … anywhere in the system if I use AI it is defaulting to that
Torq Agent, which IS Claude Code or Codex or Kimi Code, whatever the user sets up." And of TorqOS's own
loop: "it damn well better be the best in the world to justify losing Claude Code." It is not, and it
should not try to be: Claude Code, opencode and the others will keep getting better without us. What
no engine can give itself is a machine where it can be given real work safely. That is TorqOS's part.

## Facts checked before the plan was written (2026-11-15)

Tried on the check host with Claude Code 2.1.316:

- Claude Code on a **subscription login works through a plain local proxy** named by
  `ANTHROPIC_BASE_URL=http://127.0.0.1:<port>`. The proxy saw `POST /v1/messages?beta=true` with the
  login's token passing through, the answer streamed back (`text/event-stream`), and each request
  carried `x-claude-code-session-id`.
- **One long-lived headless session works**: `claude -p --input-format stream-json --output-format
  stream-json --verbose`, one question per line on its input. It kept the conversation between
  questions, ran a tool, and reported each step as a line of JSON (`system`/`init`, `assistant`,
  `user` with the tool's result, a final `result` with usage and cost).

From Anthropic's documentation (read 2026-11-15; pages under `code.claude.com/docs/en/`):

- A gateway must pass `anthropic-version`, `anthropic-beta` and `Authorization` through unchanged and
  must not buffer the stream or rewrite the body (`llm-gateway-protocol`). So the proxy is a
  byte-for-byte relay that also writes a record; it never edits a request.
- With an **API key** the proxy may hold the key and add it, so the agent's account never has it
  (`llm-gateway`). Not tried here: there is no API key on the check host.
- Settings in `/etc/claude-code/managed-settings.json` cannot be overridden by the account
  (`managed-settings`). They apply to every account on the machine that runs Claude Code.
- WebFetch runs on the machine and needs the open web; WebSearch runs at Anthropic (`tools-reference`).
- Login and token refresh go to `platform.claude.com`, not through the base URL (`network-config`).
- There is no Rust SDK; the documented way from another language is to run the CLI (`agent-sdk`).
  Claude Code does not speak the Agent Client Protocol itself; the adapter needs Node.

opencode (from its documentation and source, read 2026-11-15; nothing run):

- A provider's `baseURL` can be a local address; an API key or a local model (Ollama, llama.cpp) is
  configured there. It cannot use a Claude subscription.
- `opencode acp` speaks the Agent Client Protocol on its standard input and output, with permission
  requests sent to the client. `/etc/opencode` holds configuration and (version 2) policies the
  account cannot override.
- Two generations exist: version 1 in NixOS 26.05, version 2 (2026-10-27, not compatible) in
  nixos-unstable. The NixOS 26.05 build does not start on the check host (no AVX2).

## Anthropic's terms: what the design must respect

This is a reading of Anthropic's published pages, not legal advice. `legal-and-compliance` says a
product may not offer Claude.ai login of its own, may not route requests through a person's plan on
their behalf, and may not "collect, store, or intermediate Claude.ai credentials"; it also says nothing
there prevents "an end user from signing in to the unmodified Claude Code binary with their own Claude
subscription, including where a platform hosts Claude Code". The support article "Use the Claude Agent
SDK with your Claude plan" (updates of 2026-06-15 and 2026-10-07) says `claude -p` still draws from the
subscription's limits. From that, four rules:

1. TorqOS never ships Claude Code. The person installs it (`torq agent tools`), unmodified, and signs
   in themselves, in the agent's account.
2. The proxy passes a subscription's token through and never stores it. No record holds an
   `Authorization` or `x-api-key` value.
3. TorqOS does not remove or restrict a way of signing in.
4. The agent answers the person when the person asks. Work nobody asked for, run in the background on
   a subscription, is not built.

## The phases

Each phase is one chunk: built in small commits on its branch, every diff read by the controller, read
again by an independent reader **as a person using the system would** (nothing hidden, every command
and message understandable to someone who is not a programmer), unit tests seen failing first, and
**one** full check on the check host at the end of the phase, not before. No `unsafe`, no dead code.

### Phase 1: the engine answers
The person sets the engine, and the desktop's AI is Claude Code.

- [x] `torq agent engine claude-code` in the configuration tree (one engine per machine to begin with).
- [x] The agent's service becomes the bridge: it runs the engine as `torq-agent-<person>` in the same
      sandbox `torq agent run` uses, as one long-lived session, and speaks the same socket protocol to
      the window, the applet and `torq ask` as today (ask, stop, status, forget).
- [x] What the person sees for each step: the answer as it is written (a message at a time), each tool
      by a plain name ("ran a command", "read a file", "looked at the configuration"), a proposal with
      **Review**. (Built and unit-tested; the window and the applet are type-checked, not looked at.)
- [x] Not signed in, or Claude Code not installed: the window says so and says what to do.
- [x] Forget starts a new conversation. Stop interrupts the answer.
- [x] `claude`, typed plainly in a terminal, is the fenced one: Claude Code as your agent with the folder
      you are in shared, every word after it passed on (`torq agent plainCommand`, on by default where
      Claude Code is installed for the agents; added by the owner, 2026-11-15).
- [ ] Instructions for your agent: a text of the person's own that the engine is always given, in the
      desktop's agent and at a terminal (added by the owner, 2026-11-15). Not started.
- [x] The desktop's defaults reach a person who already has their own shortcuts or panel (the bug
      seen on the laptop: Super+Shift+A did nothing and the applet was missing). Done 2026-11-15: the key is in
      COSMIC's `defaults`; the applets by `torq-desktop-defaults` (KNOWN-GAPS 6zzt). Unit tests ran; VM test
      `tests/desktop-defaults.nix` written, not yet run.
- [x] Tests: a stand-in engine that speaks the same JSON, in unit tests and the VM tests; the real
      Claude Code driven once by hand on the check host, and what was seen written down here. (The VM
      tests `agent-service` and `desktop-agent` were run; the others that changed only evaluate so far.)
- [ ] One full check. Then the laptop, and the owner looks.

### Phase 2: TorqOS's own loop is removed
One consistent deletion, so nothing dead is left behind.

- [ ] `torq-doctor`, its loop, prompts and model client; `torq doctor`, `torq agent propose`.
- [ ] The settings under `torq agent doctor` and the local model's unit (local models return with
      opencode in phase 4).
- [ ] The account `torq-resident-<person>`, the broker's "resident rule", and session trust, which
      only the old loop could earn. Every proposal from an agent is asked, one by one.
- [ ] The tests and documents that describe them (ADR-041, 042, 051 amended; AI-LAYER.md rewritten).
- [ ] One full check.

### Phase 3: the model proxy and the audit
- [ ] `torq-model-proxy`: a small Rust service, an account of its own, listening on this machine only.
      A byte-for-byte relay that streams; it knows the calling account from the kernel, not from what
      the caller says.
- [ ] The record: for each request, the person's agent, the time, the engine's session, the model,
      the tokens in and out, and what was new in the conversation (the question, each tool's name and
      what it returned), kept where only root and the person read it, bounded in size.
- [ ] `show system agent activity` in torqcli, and the same in TorqOS Settings: what the agent did,
      asked and read, by day and by conversation.
- [ ] Claude Code is told to use the proxy by settings its account cannot change, seen only by the
      agent's account (a person's own Claude Code is not touched: later).
- [ ] The agent account's network is closed except to the proxy, sign-in, and a list the owner sets;
      the web (WebFetch) is a setting, said plainly, with what it costs the audit.
- [ ] One full check.

### Phase 4: opencode, API keys and local models
- [ ] `torq agent engine opencode`, driven through the Agent Client Protocol on its standard streams.
- [ ] The model's API key is a TorqOS secret (ADR-052): set on the Secrets page, added by the proxy,
      never in the agent's account.
- [ ] A local model (llama.cpp or Ollama) as opencode's provider.
- [ ] A build of opencode that TorqOS can pin and that runs on the check host.
- [ ] One full check.

### Phase 5: the agent in the desktop
Its place is the **TorqOS AI dock** (the owner, 2026-11-15): a slim rail on the **left edge** of the
screen that opens into a drawer, beside COSMIC's own dock and panel and not part of them. It is a
program of TorqOS's own (the toolkit TorqOS Settings uses, a layer-shell surface, its own service), so
nothing of COSMIC is changed or rebuilt, and it does not depend on the panel's list of applets. On by
default where an engine is chosen; settings for off, the edge, and hiding itself. The agent's panel
applet becomes a small mark that opens the rail, or goes; the approvals applet stays.
- [ ] The dock: the rail and its drawer, with the conversation in it (what the agent's window shows now).
- [ ] "Ask the agent" on a file or folder in Files, sharing it for that conversation.
- [ ] A quick bar: a key, a box over whatever is open, the answer in place.
- [ ] The agent's notifications (done; needs you), with their buttons.
- [ ] The screen, by the person's grant (ADR-051 6).
- [ ] One full check.

### Phase 6: the agent's workbench (the end goal; the owner, 2026-11-15)
Many views of the AI at work, in the dock's drawer: the helpers an agent started (subagents) and what
each is doing, layouts to control them, pipelines of steps, a view into the agent's own desktop. Not
planned in detail yet. What the earlier phases must leave room for, so that this is an addition and
not a rewrite:
- The socket's protocol gets a version, and its events come to carry structure: which agent or helper
  said or did a thing (the engine's own ids), not only text.
- The proxy's record keeps the engine's `x-claude-code-agent-id` and `x-claude-code-parent-agent-id`
  with each request, so a view can draw who started whom.
- Nothing is built on "one conversation for each person, for ever": the service's state and the
  protocol name a conversation, so several can come later.

### Later, not planned yet
A desk of its own for the agent (its own hidden desktop session); a pull request for the computer (a
task's files and system changes as one diff to merge or throw away); rehearsal (the agent tries a
system change in a throwaway copy of the machine); an inbox of what agents want and did; budgets the
proxy enforces; taint decided from what the proxy saw, so that trust can return; other engines (Codex,
Kimi); a person's own Claude Code outside the fence.

## Where it stands

- 2026-11-15: research done, the plan and ADR-055 written. Phase 1 started on `work/harness`.
- 2026-11-15, phase 1, the bridge, built on `work/harness` (not merged, no full check yet). What is built:
  `torq agent engine`; the agent's unit as `torq-agent-<person>` running `torq-agent-service <person>
  claude-code`; the bridge (`agent/src/service/`: `engine.rs` is all that is Claude Code's own, so opencode
  is another arm there); `claude` typed plainly. The choices are under ADR-055 ("As built, phase 1"); what
  is open is KNOWN-GAPS 6zzt.
- **The real Claude Code, by hand** (2026-11-15, the check host, Claude Code 2.1.316, signed in as the
  machine's user with a subscription). Ten short questions in all.
  - *Before the bridge was written*, the binary alone, to choose how to stop and how to tell "not signed
    in". With an empty home, `claude -p --input-format stream-json --output-format stream-json --verbose
    --permission-mode bypassPermissions` and one question on its input wrote an `assistant` line carrying
    `"error":"authentication_failed"` with the text "Not logged in · Please run /login", then a `result`
    with `"is_error":true` (its subtype still `success`), and ended with status 1: nothing was asked of a
    model. Signed in: one question answered ("OK"), a second (a long list) killed with SIGKILL six seconds
    in, then a new process with `--resume <the session the first one named>` and a third question, "which
    word did I ask you to remember?", answered with that word: a conversation survives being killed in the
    middle of an answer. `--resume` with a session that does not exist wrote a `result` with subtype
    `error_during_execution` and `"errors":["No conversation found with session ID: …"]`, and ended.
  - *Through the bridge*: the service's own code run outside systemd as the machine's user (the unit test
    binary's `by_hand` entry, `TORQ_BY_HAND=<a scratch directory>` as the agent's home and working
    directory, `TORQ_BY_HAND_ENGINE=/run/current-system/sw/bin/claude`; the sign-in reached by a link from
    that directory's `.claude` to the user's own and a copy of `.claude.json`), asked over its socket with
    `socat`. In order:
    1. `{"type":"ask","text":"What is 17 times 3? Reply with only the number."}` → `say "51"`, `done 0`;
       status then `turns 1, conversation 1`, and the session's name was in the state file.
    2. "Add 4 to that. Reply with only the number." → `say "55"`, `done 0`: the same process, the
       conversation kept.
    3. "Run the shell command `echo torq-$((6*7))` and tell me exactly what it printed" → `called "ran a
       command"`, `say "torq-42"`, `done 0`.
    4. "Write the numbers from 1 to 800 in words…", and eight seconds later `status` (`running true`) and
       `stop` from another connection → the asker got `done 137`, status `running false`, no engine
       process left.
    5. "What was the result of the very first calculation I asked you for?" → `say "51"`, `done 0`; the
       engine's process now had `--resume <the same session>` on its command line.
    6. `forget` → `forgotten`, status `turns 0, conversation 2`, no engine process; then "Have I asked
       you anything before this message in this conversation?" → `say "no"`; a new session's name in the
       state file, no `--resume` on the command line.
    7. The service killed and started again → status `turns 1, conversation 2` before anything was asked;
       "What exactly did I ask you in my previous message? Quote it." → it quoted question 6; the engine
       had been started with `--resume <that session>`. (With the service killed, the engine it had left
       ended by itself, its input closed.)
  - *Not tried with the real one*: inside the unit and its sandbox, as an agent account; TorqOS's tools
    (`torq-mcp` is not installed on the check host, so its entry in the settings pointed at nothing) and a
    proposal; attached text; the window and the applet; `claude` typed plainly; signing in through `torq
    agent run`. A client that sends its question and closes its side at once (`socat` with no input left)
    is taken as gone, and its question stopped: that is the service's rule for an asker who leaves, seen
    here first.
- **What was run for phase 1** (2026-11-15, the check host). Unit tests: agent 80 (+2 ignored: the
  tests' helper, and the by-hand entry), torqcli 471, wire 22; the broker's and the secrets daemon's ran
  as part of building their packages for the VM test. TorqOS Settings: type-checked (`nix develop
  .#torq-settings`), not started. The VM test `agent-service`: passed, after one failure that found a
  real fault (a program the engine left in a session of its own held the engine's output open, so a stop
  never ended and the program stayed: fixed, with a unit test that fails the old way). The agent unit's
  exposure by `systemd-analyze security` there: 1.8. The VM test `desktop-agent`: passed, the first time
  (COSMIC, the applet, the window opened by its key; the engine a stand-in). `agent-assertions` and `agent-units` evaluate
  (`agent-units` was not built). Not run: `nix flake check`; the VM tests `agent-mcp`,
  `agents-per-person`, `agent-proposals`, `doctor`, `secrets` and `agent-run` (the first three were
  edited and evaluate).
- **Left for phase 2, still in the tree after phase 1:** `torq-doctor` and its commands (`torq doctor`,
  `torq agent propose`, and `torq-doctor --propose --task`, which nothing starts any more); the settings
  under `torq agent doctor` (`apiKeyFile` now has no use at all) and the local model's unit; the
  `torq-resident-<person>` accounts (made where doctor mode is on, nothing runs as them) with the broker's
  resident rule and session trust; `Task` and `Turn` in `wire/src/agent.rs`; the taint words in torqcli
  and the window ("coding-tool", "not-the-resident-agent"); the Agent page's lines for doctor mode.
