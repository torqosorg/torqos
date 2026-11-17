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

What is removed (done, phase 2): TorqOS's own agent loop (`torq-doctor`, its prompts, its model client), the
one-shot `torq doctor`, the model settings under `torq agent doctor`, and the second account
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
- [x] Instructions for your agent: a text of the person's own that the engine is always given, in the
      desktop's agent and at a terminal (added by the owner, 2026-11-15). Built 2026-11-15: the person's file
      `~/.config/torq/agent-instructions.md`, `torq agent instructions [edit | clear]`, the Agent page's
      section, sent with every question and given by `claude` typed plainly. (Unit-tested; the VM test
      `agent-service` checks it with the stand-in; the Agent page is type-checked, not looked at.)
- [x] The desktop's defaults reach a person who already has their own shortcuts or panel (the bug
      seen on the laptop: Super+Shift+A did nothing and the applet was missing). Done 2026-11-15: the key is in
      COSMIC's `defaults`; the applets by `torq-desktop-defaults` (KNOWN-GAPS 6zzt). Unit tests ran; VM test
      `tests/desktop-defaults.nix` written, not yet run.
- [x] Tests: a stand-in engine that speaks the same JSON, in unit tests and the VM tests; the real
      Claude Code driven once by hand on the check host, and what was seen written down here. (The VM
      tests `agent-service` and `desktop-agent` were run; the others that changed only evaluate so far.)
- [x] One full check: passed on the check host on 2026-11-16 at bf07824 (`nix flake check`, every VM test). Then the laptop, and the owner looks (not yet).

### Phase 2: TorqOS's own loop is removed
One consistent deletion, so nothing dead is left behind.

- [x] `torq-doctor`, its loop, prompts and model client; `torq doctor`, `torq agent propose`. (Removed
      2026-11-16; the crates' unit tests ran.)
- [x] The settings under `torq agent doctor` and the local model's unit (local models return with
      opencode in phase 4). (Removed with `modules/ai.nix`; `torq agent model` too. A configuration that
      still sets one is refused in words that say what to delete: `tests/agent-assertions.nix`, evaluated.
      The journal's setting is `torq agent journal`.)
- [x] The account `torq-resident-<person>`, the broker's "resident rule", and session trust, which
      only the old loop could earn. Every proposal from an agent is asked, one by one. (Removed; the
      broker's unit tests ran, and a ledger written by the old broker still verifies.)
- [x] The tests and documents that describe them (ADR-041, 042, 051 amended; AI-LAYER.md rewritten).
      (`tests/doctor.nix` deleted; `agent-proposals`, `agent-mcp`, `agents-per-person`, `agent-assertions`,
      `agent-units` rewritten or edited: which of them ran is below.)
- [x] **Found on the laptop (2026-11-16, the real Claude Code): no agent could propose anything on a real
      machine.** A proposal was torqcli's commit as the agent, which evaluates the change (`nix eval`), and the
      Nix daemon refuses an agent's account, by design. No test saw it: no VM evaluated a real
      configuration. Added (ADR-056, KNOWN-GAPS 6zzv): the agent's torqcli evaluates nothing; the broker has
      the change checked by `torq --check-tree` as the locked account `torq-check`, in a unit of its own,
      and records it as valid, refuses it as not valid, or records it as not checked and tells the agent and
      the person. The agent stays refused at the daemon. (Built; the crates' unit tests and the VM test
      `agent-check`, with a real evaluation, ran: below. Not seen on the laptop yet.)
- [x] One full check: passed on the check host on 2026-11-16 at f82d281, on its third run (the first two, and why they failed, are under "As built"). Then the laptop, and the owner looks (not yet).

### Phase 3: the model proxy and the audit
- [x] `torq-model-proxy`: a small Rust service, an account of its own, listening on this machine only.
      A byte-for-byte relay that streams; it knows the calling account from the kernel, not from what
      the caller says. (Built 2026-11-16 on `work/harness-p3`, ADR-057: unit tests, the VM test
      `model-proxy`, and the real Claude Code through it by hand, outside a unit. Below.)
- [x] The record: for each request, the person's agent, the time, the engine's session, the model,
      the tokens in and out, and what was new in the conversation (the question, each tool's name and
      what it returned), kept where only root and the person read it, bounded in size. (Built with it;
      also the answer, the engine's helper ids, and each site asked for. Not tamper-evident: KNOWN-GAPS 6zzx.)
- [x] `show system agent activity` in torqcli, and the same in TorqOS Settings: what the agent did,
      asked and read, by day and by conversation. (Built 2026-11-17, ADR-057 amended (4): unit tests and the
      VM test `model-proxy` for the command; the Settings page type-checked only, not seen on a desktop.)
- [x] Claude Code is told to use the proxy by settings its account cannot change, seen only by the
      agent's account (a person's own Claude Code is not touched: later). (Built: the VM test has a
      stand-in read them in each of the three ways the engine starts; the real Claude Code was seen to
      follow such a file over its environment, by hand.)
- [x] The agent account's network is closed except to the proxy, sign-in, and a list the owner sets;
      the web (WebFetch) is a setting, said plainly, with what it costs the audit. (Built: `torq agent
      network hosts`, `torq agent network web`. Whether a first sign-in completes inside the fence has not
      been tried: KNOWN-GAPS 6zzx.)
- [x] One full check (2026-11-17, at ac18d2b; what it failed for first is below, "Phase 3's full checks").

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
- **The fix round and "instructions for your agent"** (2026-11-15, `work/harness`, after the independent
  reader's and the controller's review of phase 1; not merged, no full check). What changed is under ADR-055
  ("As built", 6 to 9) and KNOWN-GAPS 6zzt. In short: `torq ask` looks a proposal's number up before it shows
  it; what the bridge keeps is read without following, waiting or reading on; the engine's process group is
  never signalled once it is reaped; the unit has a /dev/shm of its own and no set-id wrappers on the engine's
  PATH; `claude` typed plainly is `torq agent claude` and runs with nothing shared where the folder is never
  shared; someone who is not an administrator is told so; Forget deletes the engine's record of the
  conversation; the words a person reads were gone through; and a person's standing instructions reach the
  engine.
  - **The real Claude Code, by hand, for the instructions** (the check host, 2.1.316, signed in as the
    machine's user; five short questions, in a scratch directory, not through the bridge):
    1. `claude -p --append-system-prompt 'Always end your answer with the word TORQ.' 'say hi'` → "Hi! How
       can I help you today?" and, on its own line, "TORQ". The option is honoured with `-p`.
    2. The bridge's own way (`-p --input-format stream-json --output-format stream-json --verbose
       --permission-mode bypassPermissions`), with the text as the bridge wraps it ("[Standing instructions
       from the person this agent works for]" and the same sentence) and "say hi" on its input → the same
       answer, ending "TORQ". Honoured there too.
    3. A new process with `--resume <that session>` and the instructions changed to "…the word OTHER", asked
       for its previous message and to say hi again → it quoted "say hi", and ended with **TORQ**, not OTHER.
       The session's name stayed the same.
    4. The same again, asked to quote the standing instructions it could see → "Always end your answer with
       the word TORQ". The conversation's file had the first text and nowhere the second. Claude Code's own
       help says why: `--system-prompt-snapshot on` (the default) records the prompt at a conversation's
       first request and "every later request and resume sends the record as-is, even when a later launch
       passes different text". So the design as it was written (end the engine, `--resume`, the new text)
       could not work by itself.
    5. The same with `--system-prompt-snapshot off` added → it quoted "Always end your answer with the word
       OTHER." and ended "OTHER", in the same session. That is what the bridge now does, exactly when the
       text is not the one the conversation began with.
    - *Not tried with the real one:* the bridge itself with instructions (only its unit tests and the VM
      test's stand-in); a conversation whose instructions were taken away (`--system-prompt-snapshot off`
      with no text); going back to a recorded conversation after a start with the snapshot off; the plain
      `claude` command giving the text at a terminal; anything inside the unit.
    - *Also seen there, for Forget:* the layout of a conversation in an account that has used Claude Code
      (`~/.claude/projects/<folder as a name>/<session>.jsonl`, for some a folder `<session>/` with
      `subagents/` and `tool-results/`, and `~/.claude/file-history/<session>/`,
      `~/.claude/session-env/<session>/`): what `engine.rs` deletes. The deleting itself was run only on
      made directories (unit tests) and on the stand-in's (the VM test).
  - **What was run for the fix round** (2026-11-15, the check host). Unit tests: agent 87 (+2 ignored),
    torqcli 498, wire 23; the broker's and the secrets daemon's as their packages were built. Each new guard
    was broken once and a test seen to fail (the commits say which). TorqOS Settings: type-checked, not
    started. `agent-assertions`: builds (it only evaluates), and fails with the unit's /dev/shm taken out, with
    `MemoryDenyWriteExecute` put in, and with the name's limit measured the old way. The VM test
    `agent-service`: passed, 27 subtests, on its second run (the first stopped at an expectation of the
    test's own: exactly two fields in what the service keeps, where there are now three). In it, with the
    stand-in: the unit's exposure by `systemd-analyze security` is still 1.8; from inside the unit the
    engine's PATH is its own profile and the system's, /dev/shm is empty and its own (a file another account
    left in the machine's is not there), the state directory is writable to it, tracing its parent is refused
    (errno 1) and so is opening its parent's memory, while its own child's opens; forget deletes the
    stand-in's record; `claude` in the home, in /etc, in ~/.ssh and in ~/.config/x runs with nothing shared
    and says the one line; a non-administrator is told so; a person's own `claude` is found first; a second
    `claude` is told to close the first; the instructions reach the engine and a changed text restarts it
    with `--resume` in the same conversation. The VM test `desktop-agent`: passed, 11 subtests, the first time, once the Settings package built (its dependency hash was out of date for its lock file, see the commit; COSMIC's text editor is on the machine). Evaluated only (their
    derivations instantiate): `agent-mcp`, `agent-units`, `agents-per-person`, `agent-proposals`, `agent-run`.
    Not run: `nix flake check`; every other VM test; `agent-units`' build; the VM-level guards were not broken
    to see the VM test fail (the unit tests and `agent-assertions` were).
- **Left for phase 2, still in the tree after phase 1** (all removed in phase 2, below)**:** `torq-doctor` and its commands (`torq doctor`,
  `torq agent propose`, and `torq-doctor --propose --task`, which nothing starts any more); the settings
  under `torq agent doctor` (`apiKeyFile` now has no use at all) and the local model's unit; the
  `torq-resident-<person>` accounts (made where doctor mode is on, nothing runs as them) with the broker's
  resident rule and session trust; `Task` and `Turn` in `wire/src/agent.rs`; the taint words in torqcli
  and the window ("coding-tool", "not-the-resident-agent"); the Agent page's lines for doctor mode.
- 2026-11-16, **phase 2, the old loop removed**, built on `work/harness-p2` (not merged, no full
  check yet). What went, what stayed and why is under ADR-055 ("As built, phase 2"); what is left open is
  KNOWN-GAPS 6zzu. In short:
  - *Gone:* `torq-doctor` and everything of it in the agent crate; `torq doctor`, `torq agent propose`,
    `request system agent trust`; `modules/ai.nix` (`torq agent doctor …`, `torq agent model …`, llama.cpp's
    unit); the accounts `torq-resident-<person>`; at the broker the resident rule, the model's judgement and
    session trust; `Task`, `Turn` and the `tainted` event on the agent's socket; the reserved unit name
    `torq-agent-key@`; `Brain` (now `Engine`: what answers, and whose service it asks).
  - *Every proposal waits.* A session of an agent's is marked from its start and a proposal of an agent's is
    marked whatever it names; nothing applies one without a person. A person reads "Your agent proposed
    this" and what it had read, in words, at the apply prompt and on the Proposals page; "proposed by an
    agent" in the ledger's view. The ledger's own field keeps its name (`tainted`): its bytes never change.
  - *Kept, to read what older machines hold:* the ledger's old words verify (a fixture written by the broker
    of 74339a1, `wire/tests/fixtures/before-phase-2.jsonl`); a session record from before is read, and is
    tainted; torqcli says the old session words as it says the new one.
  - *A machine that still sets a removed setting* is told what is set, in which file, and what to type
    (`delete torq agent doctor`, in the commit that updates TorqOS), not that an option is unknown; the
    journal's new setting is a second commit after the update.
  - *Secrets say only what they do* (decided by the owner and built the same day; ADR-052's amendment of
    2026-11-16, KNOWN-GAPS 6zzu). Their rules told a clean session from a tainted one, and no agent has a
    clean one now. So: two words for when a person is asked (`each-use`, `without-asking`; a stored
    `first-use` or `never` reads as `each-use`, `never-even-tainted` as `without-asking`), and one kind of
    grant (one named secret, for minutes). `allow all`, `allow <name> session` and `tainted-too` are gone
    from the broker, torqcli, the dialog and the Secrets page; a person reads why they are asked in one
    sentence, and no word about sessions or taint.
- **What was run for phase 2** (2026-11-16, the check host, on `work/harness-p2`). Unit tests, each
    crate's whole suite: agent 59 (+2 ignored: the tests' helper, and the by-hand entry), torqcli 495, wire 23,
    secrets 15, broker 319 (across its test binaries; they include every ledger fixture read and written back
    byte for byte, the new one among them). TorqOS Settings: type-checked (`nix develop .#torq-settings`), not
    started. Each new guard was broken once and a test seen to fail, then put back: in the broker, a record
    from before read as tainted, an agent's proposal marked with and without a session, a session marked at
    its start, an old grant of trust read as no grant, another agent's session refused; in torqcli, the old
    loop's words kept out of what a person reads; in the modules, the removed option's words and the
    journal's group (`agent-assertions` threw each time). Every check of the flake evaluates
    (`nix eval .#checks.x86_64-linux`, each one's derivation: 37 checks; `doctor` is gone). VM tests, three:
    `agent-proposals` passed, 7 subtests, on its second run (the first stopped at the test's own date for a
    `commit at`, further off than a schedule may be); `agent-service` passed, 27 subtests, the first time
    (the real torq-mcp through the bridge: the proposal is alice's agent's, marked, waiting; nothing of the
    old loop in the schema, on the disk or in /etc/passwd); `agent-mcp` passed, 6 subtests, on its second run
    (the first stopped at an expectation of the test's own). And `agent-units` (no VM: the check on the
    agent's units as built, every case and the flake's own hosts) built, on its second run (the first
    stopped at one case of the test's own, a link to nothing named as an agent's unit, which is refused under
    both rules). Not run: `nix flake check`; every other VM test (`agents-per-person`, `secrets`, `desktop`,
    `desktop-agent` and `agent-run` were edited or touch what changed, and only evaluate); nothing on a
    machine that has the old settings or an old ledger (the fixture stands in for the ledger; the removed
    settings' refusal is held at evaluation only); nothing on a desktop.
- **What was run for "secrets say only what they do"** (2026-11-16, the check host, on `work/harness-p2`,
    after the above). Unit tests, each crate's whole suite: torqcli 501, broker 317 (across its test
    binaries, `trust_loop` among them), wire 24, secrets 15, agent 59 (+2 ignored). TorqOS Settings:
    type-checked, not started; the Secrets page's new rows and the dialog's new line have been drawn nowhere.
    Each guard was broken once and a test seen to fail, then put back: in the wire crate, a stored `never`
    and a stored `first-use` read as `without-asking`; in the broker, a grant read without its
    `tainted_too`, and one read with a `scope`, the rule's word ignored (`asks` always no), and an old
    waiting-use file read with its `tainted` and `why` left in; in torqcli, an answer the dialog does not
    offer taken, an allowance offered where the agent may not use the secret, where the rule never asks and
    where one holds already, a grant in the broker's answer read without whose secret it is, an allowance
    taken to cover a secret by its name alone, and the dialog's "you already allow this" found by name
    alone. (One more was tried and no test failed: reading the old shape's name alone still lists nothing,
    because whose secret it is cannot be read from that shape either.) VM tests: `secrets` passed, 16
    subtests, on its third run (the first never started: the broker's package runs its tests when it is
    built, and two of `trust_loop`'s still said the old word, since rewritten; the second stopped at an
    expectation of the test's own, the exit code of an apply that needs a password). Four more whose
    expectations changed: `agent-proposals` (7 subtests), `agent-mcp` (6) and `agents-per-person` (7) passed
    the first time (a session's record no longer has `tainted` beside what it read); `desktop` (13 subtests) passed the first time (the applet sees a waiting use whose facts have no `tainted`).
    Every check of the flake evaluates. Not run: `nix flake check`; `desktop-agent` and every other VM test
    (not edited); anything on a machine with rules or a ledger from before (the wire crate's units and the
    broker's fixture stand in); anything on a desktop.
- 2026-11-16, **an agent's proposal is checked by the broker's checker** (ADR-056, KNOWN-GAPS 6zzv; built on
    `work/harness-p2`, not merged, no full check). Found on the owner's laptop with the real Claude Code: no
    agent could propose anything on a real machine (torqcli's commit as the agent ran `nix eval`, and the Nix
    daemon refuses an agent's account). Now the agent's torqcli evaluates nothing; the broker, after its own
    vetting, has the git tree the change leaves evaluated by `torq --check-tree` as the locked account
    `torq-check` in a unit of its own, and records the proposal as valid, refuses it as not valid, or records
    it as not checked and tells the agent and the person. The agent stays refused at the daemon; `torq-check`
    is the one account added to `allowed-users` (never trusted).
- **What was run for it** (2026-11-16, the check host, on `work/harness-p2`). Unit tests, each crate's
    whole suite: torqcli 505, broker 334 (across its test binaries, `trust_loop` among them; 10 of them the
    new `tests/check.rs`, with stand-in checkers), wire 24, secrets 15, agent 59 (+2 ignored). TorqOS
    Settings: type-checked, not started (the Proposals page's new line has been drawn nowhere). Every check
    of the flake evaluates (38: `agent-check` is new). VM tests, one at a time: `agent-check` (new) passed,
    4 subtests, on its third run (the first stopped at an expectation of the test's own, the list of trusted
    users written twice; the second at torqcli itself refusing, before anything is proposed, a name that is
    no package at all, so the test now names one only the evaluation knows is none); `agent-proposals`
    (7 subtests), `agent-mcp` (6) and `agent-service` (27) each passed on their second run (the first of
    each stopped where the first proposal's check took half a minute: with the little room a test VM's store
    has, Nix collected garbage before each check; they set `nix.settings.min-free = 0` now, as the image's
    test does). In `agent-check` the evaluation is real and offline: the first check took 78 seconds in the
    VM (29 of CPU, a gigabyte at its peak), and 4 minutes 10 in the run before, with `fsync-store-paths` on,
    which the test turns off for its VM. Each guard was broken once and a test seen to fail, then put back.
    In the VM test: the agent's account added to `allowed-users` ((a) fails at the list, and with that
    assertion out too, at the agent's `nix store info` succeeding); the checker's verdict thrown away
    ((b)'s refusal). In the broker's units: the check started before the policy's vetting, the broker's lock
    held through the check, the tree's comparison dropped, the refusals' gate dropped, `--check-cmd` taken
    from a caller, the configuration as it is not asked before "not valid", the checker's output read
    without a bound, and a "not valid" recorded instead of refused (that one fails the broker's own tests
    when its package is built, before any VM starts). In torqcli: the agent's commit evaluating first, and
    the checker answering "not valid" for what kept it from evaluating at all. On the check host itself the
    final unit's properties were run with the real `torq --check-tree` on this machine's own configuration
    (as the user `viper`: there is no `torq-check` here; the account's place at the daemon is the VM test's):
    `{"ok":true,"warnings":[…]}` in 15.7 seconds, a gigabyte at its peak, nothing dropped from the list; and
    on a tree with a package that does not exist, `{"ok":false,"why":"\"nosuch-package-xyz\" isn't a
    package."}` in 5 seconds. Not run: `nix flake check`; every other VM test; anything on the laptop, where
    the fault was found; two agents proposing at once; a machine whose TorqOS comes from a folder in a home.
- 2026-11-16, **the check's fix round, after an independent review and a failed full check** (ADR-056 as
    amended, KNOWN-GAPS 6zzv and 6zzw; on `work/harness-p2`, not merged). The review found that a failure
    that was not the change's was taken for "not valid" and, with the kept verdict on the configuration
    (`check-base`), turned into refusals counted against the agent; that the `torq-check` assertion held
    less than it said; four wordings; and that ADR-056 claimed too much of what an agent's data can do.
    The full check of c866781 failed in four VM tests (`agent`, `agent-run`, `agents-per-person`, `socket`)
    that the round before had not rerun: an agent's proposal there starts the real checker, Nix collected
    garbage before the checker's copy into the store (a test VM is always below `min-free`), and the
    test's client gave up after 30 seconds. Measured in a VM made for it: the same evaluation of a stub
    repo, 40.0 seconds with TorqOS's settings, 0.4 with `min-free = 0`, 37.5 with only `fsync-store-paths`
    off. All seven agent tests that propose now set `min-free = 0` and assert the check's outcome.
- **What was run for it** (2026-11-16, the check host, at cb9662a; this entry is the only change
    after it). Unit tests, each crate's whole suite, in the dev shell: torqcli 506, broker 339 (13 of them
    `tests/check.rs`), wire 24, secrets 15, agent 59 (+2 ignored). Every check of the flake evaluates
    (their `drvPath`s, 38). `checks.agent-assertions` built. VM tests, `--max-jobs 2`, each passed on its
    first run: `agent`, `agent-run`, `agents-per-person`, `socket`, `agent-check`, `agent-proposals`,
    `agent-mcp`, `agent-service`. Each new guard was broken once and a test seen to fail, then put back.
    In the broker: the verdict kept per commit again (two tests fail), one deadline shared by both runs,
    control characters only replaced in the checker's words, a busy check ledgered as a refusal, the
    check lock's own words taken out. In `modules/broker.nix`, each clause of the assertion taken out and
    its case evaluated: the three of `trusted-users`, `hashedPassword`, `password`, `hashedPasswordFile`,
    `initialHashedPassword`, the shell, the home, `createHome`, the SSH keys and key files, and the
    membership of another group each stop being refused. Two clauses are second guards that no case
    proves alone: `initialPassword` (NixOS makes it the `password` where users are not mutable, so that
    clause refuses it) and `extraGroups` (NixOS adds the account to the group's members, so that clause
    does). Not run: `nix flake check`; any other VM test; the checker meeting a full disk, a lost daemon
    or a signal for real (the unit tests give `nixerr::outside` Nix's words as Nix 2.34 writes them; the
    VM test has the daemon refuse the checker's account, and the checker says so); a widened
    `mayPropose.settings`; anything on the laptop.
- **Phase 2's full checks** (2026-11-16, the check host, `nix flake check --keep-going --max-jobs 2`). Three runs.
    At c866781: failed in four agent VM tests (above). At 04a87e0: failed in two. `installer` was cut off by its
    own one-hour limit in its last step, and again when run alone: the test takes 3530 to 3650 seconds there, so
    its limit is now ninety minutes (f82d281, the only change after 04a87e0). `desktop-defaults` waited 90
    seconds for the agent's applet after login and did not see it; run alone at the same commit it passed, the
    applet up in 14 seconds, and the cause was not found. At f82d281: every check passed (everything but
    `installer` was already built from the run before, `desktop-defaults` from its run alone). Merged to `main`
    after that. Not run on any machine yet: the proposal checker with a real configuration outside a VM.
- 2026-11-16, **phase 3, its first half: the model proxy, its record, and the agents' closed network**, built
    on `work/harness-p3` (ADR-057, KNOWN-GAPS 6zzx; not merged, no full check). The second half, showing the
    record (`show system agent activity`, the page in TorqOS Settings), is not built. In short:
  - *The proxy* (`proxy/`, `modules/model-proxy.nix`): `torq-model-proxy`, a locked account of its own,
    listening on 127.0.0.1 port 7801. A request for the model goes to `api.anthropic.com` as it came (hyper
    and rustls; certificates checked against the machine's authorities) and the answer streams back. The
    caller is named from the kernel's table of connections, as the broker names the secrets daemon's: one
    port for every agent, not one each (ADR-057 4 says why).
  - *The record* (`wire/src/activity.rs`): `/var/lib/torq-model-proxy/<person>/<UTC day>.<part>.jsonl`, a line
    for each request (whose agent, when, the engine's session and helper ids, model, tokens, how it ended,
    what was new, what was answered) and for each site asked for. Bounded, and it says where it cut. The
    person reads it by an access rule; no agent's account reads or writes it. Not in the ledger.
  - *The fence*: a table of the kernel's packet filter by the account a socket belongs to
    (`torq-agent-network.service`); nscd and systemd-resolved closed to the agents; nothing started as an
    agent while it is not up.
  - *Sites*: `torq agent network hosts` (a name, or `*.name`) and `torq agent network web` (off), through
    the proxy as HTTPS only; `platform.claude.com` always (signing in).
  - *Claude Code's settings for the agents*: `/etc/torq/claude-code/managed-settings.json`, shown as
    `/etc/claude-code` only inside an agent's sandbox, and the same values in the environment.
- **The real Claude Code, by hand, for phase 3** (2026-11-16, the check host, 2.1.316, signed in as the
    machine's user; eight short questions in all, each `claude -p --model haiku`, in a scratch folder, inside
    a network of its own (`unshare -rn`: nothing but a loopback) with one door to a proxy outside it).
    Nothing here printed or kept the sign-in: the measuring proxy wrote down hosts, paths and the names of
    headers only.
  - *What Claude Code contacts* (four questions through a measuring proxy written for it, which let every
    site through or refused them):
    1. Everything allowed, "Reply with only the word OK.": one `POST /v1/messages?beta=true` to the base
       address (the answer `text/event-stream`, `content-encoding: gzip`); and by `CONNECT`, past the base
       address: `api.anthropic.com:443` (16 connections), `mcp-proxy.anthropic.com:443` (4: the claude.ai
       connectors), `http-intake.logs.us5.datadoghq.com:443` (1: usage reports), and `mcp.zoom.us` and
       `mcp-us.zoom.us` (3: a plugin of this user's own, not Claude Code's).
    2. Every `CONNECT` refused: the question is still answered. Asked which MCP servers it had tools from,
       it said none.
    3. Only `mcp-proxy.anthropic.com` allowed: still none. So the connectors need `api.anthropic.com`
       directly as well (where it learns which the account has), and `mcp-proxy.anthropic.com` to use them.
    4. Every `CONNECT` refused and `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`: answered, and only two
       attempts at `api.anthropic.com` instead of thirteen, none at the usage reports' host or the plugin's.
    - *Not seen:* a sign-in or a renewal of one (none happened in these runs); an update check (this Claude
      Code is NixOS's package). From the program's own text, not from the network: the sign-in is exchanged
      and renewed at `https://platform.claude.com/v1/oauth/token`, a person signs in on a page at
      `claude.com/cai/oauth/authorize` or `platform.claude.com/oauth/authorize` (their own browser), and
      after a sign-in it asks `api.anthropic.com/api/oauth/…`; updates are named at `downloads.claude.ai`
      and `storage.googleapis.com`; usage reports also at `api.anthropic.com/api/event_logging/…`.
  - *Through TorqOS's own proxy* (the crate's by-hand entry, `cargo test --test proxy by_hand -- --ignored`:
    the real service, the machine's authorities, whoever runs it as alice's agent; four questions):
    5. "Run the shell command: echo torq-$((6*7)) and tell me exactly what it printed." → "The command
       printed exactly: `torq-42`". The record: two request lines (the model named, status 200, complete,
       tokens with the cache's counts, `stop` `tool_use` then `end_turn`; the first's answer the tool call
       `Bash` with its input, the second's new part the tool's result `torq-42` named `Bash` and its answer
       the text), and a line for each refused site (`api.anthropic.com` as `the-model`, the others
       `not-listed`). No credential in the file (searched for).
    6. and 7. The first request's new part was empty: Claude Code 2.1.316 ends a request with a message of
       its own (role `system`, some 30 KB) after the person's. Two more questions to see the shape (roles and
       lengths only); the record now takes everything after the model's last message.
    8. The settings an account cannot change: in a mount namespace with `/etc/claude-code/managed-settings.json`
       holding the proxy's address in its `env`, and the environment naming another address
       (`ANTHROPIC_BASE_URL=http://127.0.0.1:9`) and no proxy at all: the question was answered through the
       proxy, the refused sites were asked of it, and the record's new part held the question. So the
       managed file's values are the ones used.
  - *Not tried with the real one:* inside the agent's unit or under `torq agent run` with the fence (the VM
    test has a stand-in there); a sign-in from inside the fence; a claude.ai connector used; a long session;
    more than one conversation at once.
- **What was run for it** (2026-11-16 and the night after, the check host, on `work/harness-p3`; no full check).
    Unit tests in the dev shell, each crate's whole suite: wire 28, torq-model-proxy 38 (+1 ignored: the
    by-hand entry; against a stand-in for the model's service that speaks TLS with a test authority's
    certificate), agent 60 (+2 ignored). The broker's `agent::` tests by hand (7); its whole suite, torqcli's
    and the secrets daemon's ran as their packages were built for the VM tests (their only change is
    torq-wire, and one test in the broker). The proxy's package builds in the Nix sandbox with its tests.
    Every check of the flake evaluates (39: `model-proxy` is new). `agent-assertions` built (the proxy's
    account and unit among its cases). VM tests, `--max-jobs 2`:
  - `model-proxy` (new, two machines): passed, 11 subtests, on its eighth run. The runs before stopped at:
    the test's own node settings (once); a person's zsh reading an IPv6 address's brackets as a pattern;
    `rm` asking a question nobody answered; the test's expectation of how an access rule is printed; **the
    record's files readable by the proxy's own group through the rule new files inherit** (no one is in
    that group; the rule is now said whole); **the engine in the agent's service not told of the proxy**
    (the bridge makes its environment from nothing: fixed in `agent/`, with a unit test); and the test's
    own way of keeping the fence down. The proxy's unit: exposure 1.4 by `systemd-analyze security`.
  - `agent-service` (its unit changed): passed, 27 subtests, on its second run (the first stopped at the
    test's own list of the engine's environment, which now has the proxy's five names). `agent-run` (the
    launcher changed): passed, 9 subtests, the first time. `agent-units` and `first-build-offline` (no VM)
    built.
  - Each guard was broken once and a test seen to fail, then put back. In the crates (unit tests): a caller
    that is no agent taken for one; a credential put in the record; the query kept in the path; the
    connection's own headers passed on; the host not renamed; listening off this machine; the record not
    asked before passing; no line for an engine that goes away; the answer held to its end; the model
    allowed as a site; this machine's addresses reached; a private address under the web's rule; an IPv6
    address that maps an IPv4 one; any port; a loose `*.name`; the sign-in's name matched loosely; an
    account that is no system account; each bound (things, a request, an answer, a text, a file, a
    person); lost lines not counted; no limit at once; no authority taken for every authority; a tool's
    name not found by its call; only the last message read; an unknown word in the settings; the engine
    not told of the proxy, and told everything. In `modules/model-proxy.nix` (each clause of the account's
    assertion taken out and its case evaluated, and the packet filter emptied whole): all stop being
    refused but three that are second guards no case proves alone (`isSystemUser`, `extraGroups`,
    `initialPassword`). In the VM test (twelve runs, each failing in the subtest that holds the claim): the
    fence's rule; its refusals; nscd and systemd-resolved not hidden, under `torq agent run` and in the
    service; the system bus open at systemd-resolved; a person's record open to everyone; the person's
    access rule gone; the launcher and the service without the fence; Claude Code's settings not shown,
    under `torq agent run` and in the service; and those settings put where every account reads them.
  - *Tried and no test failed:* a check that a connection is from this machine's own address, beside the
    kernel's table (which already holds only such connections): taken out as a guard that proved nothing.
  - *Not broken to see a test fail:* rustls's checking of certificates itself (the tests show a stranger's
    certificate and a wrong name refused); the proxy's unit hardening line by line.
  - *Not run:* `nix flake check`; every other VM test (`secrets`, whose agent uses the secrets daemon's
    port, which the fence lets through, only evaluates; so do `agent`, `agent-mcp`, `agent-proposals`,
    `agents-per-person`, `desktop-agent`); the real Claude Code inside the fence; a sign-in; a machine with
    NixOS's nftables firewall turned on; an agent holding many large requests at once.
- **What was run for the security reader's round** (2026-11-17, the check host, on `work/harness-p3`; ADR-057,
  amended, 3; KNOWN-GAPS 6zzx; no full check).
  - *The reruns owed by the port and spoofing fixes:* `agent-service` and `agent-run` passed at 0167349. `secrets`
    failed: its agent signs in over SSH, and the fence refuses the agent's account every connection but the
    proxy's and the secrets daemon's. The test now has root make that connection for it (a unix socket only the
    test opens) and checks that the agent's own `ssh` is refused; it also holds its own daemon's refusal of a
    caller on 127.0.0.2. Passed.
  - *Built:* polkit's rule refusing the agents everything, linger off for every agent, `torq agent run` in a
    scope of its own in `torq-agents.slice` (which the fence binds), its seccomp filter for socket families,
    `torq-agent-fence-check` before the agent's service and `torq agent run`, a failed reload stopping the
    fence; in the proxy, a non-agent answered and closed at once, `HEAD_WAIT`, the connection bounds, the
    record's parts past 999, more private addresses and the machine's own networks, the model's refusal's
    words; the `web` setting's description. **Found on the way:** the kernel's table can show one socket twice
    while it changes, and since the spoofing fix that made an agent's request no one's (tests/proxy.rs failed
    about one run in forty with the new connection tests beside it; 0 in 150 after the fix); the same fix went
    into the broker's lookup for the secrets daemon. `systemd-run --scope` expands `${…}` in the command it
    is given unless told not to (`--expand-environment=no`): the first VM run of the launcher stopped there.
  - *Unit tests, each crate's whole suite, in the dev shell:* torq-model-proxy 49 (+1 ignored), broker 341,
    secrets 15, wire 28, agent 60 (+2 ignored). Each new proxy test was seen failing before its fix (the
    record past part 999, the addresses, the model's refusal's words, a non-agent closed at once, a slow head
    and an idle connection, the connection bounds, a socket shown twice); the escaped-size test passed before
    any change (the size was already counted as written) and is kept as a guard.
  - *VM tests, `--max-jobs 2`:* `agent-run` passed on its ninth run with the new subtests (socket families;
    polkit from a real console session, the agent and polkit's rule alone; the fence by its table; the fence
    stopped ending a session). The runs before stopped at: shellcheck's words on the fence check (twice);
    `systemd-run` expanding the launcher's variables; logind's JSON not naming a session's state or terminal (twice); a long line typed
    at the console losing keys (twice); the console's first prompt scrolled away by kernel messages. `agent-service`
    passed on its second run (the first: `nft` not in the test machine). `secrets` passed. `model-proxy`
    passed on its second run: the first stopped because the test's "internet" site was on the network the
    machine itself sits on, which the web setting now refuses; the site is now past a router, and the test
    checks that an address of the machine's own network is refused with the web on. `agent-assertions`
    built (linger, polkit's rule changed or taken away, the unit's fence check).
  - *Each guard broken once, a test seen to fail, then put back:* in the crates, a socket shown twice counted
    as two (proxy and broker), the connection bounds, the head's timeout, the machine's own networks, 6to4,
    NAT64, the record ordered by name, a line's size not counted. In VM tests, each in a worktree of its own
    from the commit: polkit's rule matching no agent (`agent-run`, the polkit subtest); no scope (the same, the
    agent's cgroup in the person's session); no seccomp filter (the socket families subtest); the launcher
    without the fence check (the fence subtest); the agent's service without it (`agent-service`, its fence
    subtest).
  - *Not broken to see a test fail:* the non-agent's closing at once (seen failing before the fix only); the
    failed reload stopping the fence (no test makes a reload fail); linger off (an assertion's case, not run
    on a machine); the slice ending a session at a terminal when the fence stops (its subtest passed, no
    mutation run).
  - *Not run:* `nix flake check`; every other VM test (`agent`, `agent-mcp`, `agent-proposals`,
    `agents-per-person`, `desktop-agent` only evaluate); a desktop's polkit and NetworkManager; a reload that
    fails for real; one memory budget for the proxy (not built).
- 2026-11-17, **phase 3, its second half**, built on `work/harness-p3` (ADR-057 amended (4); KNOWN-GAPS 6zzx;
  no full check): `show system agent activity` (by day, one day, one conversation), TorqOS Settings' Agent
  Activity page, a person's standing instructions handed to Claude Code as a file, and the claude.ai connectors
  said plainly on the Agent page and in the user guide.
- **What was run for it** (2026-11-17, the check host, on `work/harness-p3`; no full check).
  - *Claude Code 2.1.316, by hand, signed in to nothing:* `--help` names `--append-system-prompt[-file]`;
    `--append-system-prompt-file /nonexistent` says "Append system prompt file not found", an unknown option
    "unknown option". With `ANTHROPIC_BASE_URL` at a `socat` listener on 127.0.0.1 and a made-up key, the
    request it sent held the file's words in its system prompt; with a person's own `--append-system-prompt`
    after it, both, the file's first.
  - *Unit tests, each crate's whole suite, in the dev shell:* torqcli 517, agent 60 (+2 ignored), wire 29,
    torq-model-proxy 48 (+1 ignored; one test moved to wire with the names it tests). The broker's `secrets`
    test binary, twice, 29 of 29 each (see below). `settings/` type-checked (`nix develop .#torq-settings`,
    `cargo check`): no errors, no warnings; its first check found one (a widget's arguments).
  - *VM tests, `--max-jobs 2`, at a478f0e (built from the commit, not the working tree):* `model-proxy` passed
    on its first run, with its new subtest (the command as alice, bob, carol and alice's agent).
    `agent-service` passed on its second run at that commit; the first never started: the broker's own unit
    test `an_answer_in_the_last_moment_is_still_collected` failed inside its Nix build (rebuilt because
    torq-wire changed). Earlier, at 6d12639, `an_agent_asks_at_most_thirty_times_an_hour_across_its_sessions`
    failed the same way. Both are timed tests; the machine's load was about 7 then; neither failed in the dev
    shell. Before those, two runs of `agent-service` stopped at the test itself (its dicts compared whole;
    a new file not yet added to git).
  - *Each guard broken once, a test seen to fail, then put back:* in the crates, the record's files ordered by
    name (`parts_past_999_are_read_in_their_order`), the record's text not cleaned, and not under its `|`
    (`the_records_texts_cannot_pass_for_torqos_lines`), an agent's account not refused, the service's
    instructions file 0644, and the text on the engine's command line beside the file
    (`standing_instructions_start_the_engine…`). In a VM test, from a throwaway commit (3ffc163, reset): the
    launcher's copy made with umask 022 (`agent-service`, its instructions subtest, failed on the file's mode).
  - *Not run:* `nix flake check`; `agent-run`, `desktop-agent` and the other VM tests; the Settings page on a
    desktop; the command against the real Claude Code's record.
- 2026-11-17, **the security reader's second round** (of `0167349..be95753`), on `work/harness-p3` (ADR-057 amended
  (5); KNOWN-GAPS 6zzx; no full check): the record keeps the engine's instructions, marks a rewritten
  conversation, keeps a text document's words and says a message it cannot read; one 512 MiB budget for what it
  holds; a caller named by `NETLINK_SOCK_DIAG` (`torq_wire::peer`, shared by the proxy and the broker);
  `torq-agent-scope` (Landlock's abstract-socket and signal scopes, and the families' filter in the service);
  `socketpair` of unix only; avahi refused; the launcher's leaked descriptors closed; a refusal that no longer
  tells the agent the commands; `torq agent run` saying when the fence ended it; NAT64 under an operator's
  prefix written down.
- **What was run for it** (2026-11-17, the check host, on `work/harness-p3`; no full check).
  - *Unit tests, each crate's whole suite, in the dev shell:* torqcli 518, broker 338 (three tests of the old
    table lookup went with it), wire 33 (`peer`: crafted answers, and the real kernel: a connection, a listener
    asked as one, the same ports from 127.0.0.2), secrets 15, agent 60 (+2 ignored), torq-model-proxy 53 (+1
    ignored). `settings/` type-checked (`cargo check`): no errors, no warnings. The broker's timed tests did not
    flake this time.
  - *Each guard broken once, a test seen to fail, then put back:* in the crates, the rewrite mark set to never,
    the budget never refusing, the instructions not recorded, a message that does not read not said (proxy);
    the established-state check and the source address check in `peer::answer` (wire). In VM tests, each from a
    commit in a worktree of its own (removed after): the launcher without `torq-agent-scope` (`agent-run`, the
    abstract socket reached); the scope without the abstract-socket scope and the filter without its
    `socketpair` rule (`agent-run`: a TIPC pair made; `agent-service`: root's abstract socket reached and a TIPC
    pair made in the service, which shows systemd's `RestrictAddressFamilies` does not judge `socketpair`); the
    launcher without its loop that closes descriptors (`agent-run`: the person's two leaked descriptors of
    `/home/dev/secret` inside); avahi's bus name not denied (`agent-run`: answered); the fence's message taken
    out (`agent-run`: not said). One mutation proved nothing: the service without `--families` failed at the
    test's assertion on the unit's text, before it ran anything.
  - *VM tests, `--max-jobs 2`:* `agent-service` passed on its second run (302c4aa; the first was stopped when
    `agent-run` failed beside it, after its new subtest had passed). `agent-run` passed on its third run
    (b3bfe0b): the first stopped at the signal check (a process outside its process namespace, not there for
    it: the check is now to bubblewrap's first process, pid 1), the second at the descriptor check (a pipe of
    bubblewrap's on 3: the check is now that no descriptor is a file of the person's). `model-proxy` passed on
    its second run (b3bfe0b; the first: a later subtest counted alice's lines from the start of the test).
    `secrets`, `agent-units` and `agent-assertions` passed at 60685bf (`agent-assertions` first failed on its
    held text of the service's `ExecStart` and hidden paths, updated).
  - *Not run:* `nix flake check`; a kernel without Landlock's sixth version (the refusal there is written, not
    seen); `claude` typed plainly with a leaked descriptor (the same inner script as `torq agent run`, tested
    there); a desktop's Xwayland socket, geoclue2 and NetworkManager; the real Claude Code's requests through the
    rewrite mark (its own shortening of old tool output may mark requests: not measured); `desktop-agent` and the
    other VM tests.
- **Phase 3's full checks** (2026-11-17, the check host, `nix flake check --keep-going --max-jobs 2`). Two runs,
    and one test alone. At 8e63749: failed in `torq-settings` only, its vendored crates' hash out of date after
    `torq-wire` took `rustix` for the caller's lookup, and in the five checks that need it (`desktop`,
    `desktop-agent`, `desktop-defaults`, `tunnels`, `agent-units-its-own`); every other check passed, `installer`
    among them. The hash was set (ac18d2b, the only change after 8e63749). At ac18d2b: every check passed but
    `desktop-agent`, whose owner was still at the greeter after the password was typed twice (no panel within 30
    seconds; the greeter came up about four minutes after boot, under the run's load). Run alone at the same
    commit it passed (278 seconds). Merged to `main` after that.
