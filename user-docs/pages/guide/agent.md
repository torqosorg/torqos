# The agent

TorqOS does not come with an AI of its own. You choose a coding tool that already exists (today: Claude
Code), and it becomes **your agent**: the one program that answers wherever TorqOS offers AI. TorqOS is the
system around it. It gives the agent an account of its own, keeps it out of your home, and lets it change
the machine in one way only: by *proposing* a change, which you read and approve.

Each person has an agent of their own: an account, `torq-agent-<you>` (alice's is `torq-agent-alice`),
its home `/var/lib/torq-agent-<you>`, which no other agent can read. It proposes packages only for you,
never for anyone else, and settings only as the owner's list `torq agent mayPropose settings` allows (none,
by default). Who has an agent is `torq agent people` (the owner, by default); `torq agent run` says so
plainly if you have none.

!!! warning "Experimental, and not in a release yet"
    The agent as described here is on TorqOS's development branch. Claude Code has been driven through
    the agent's service once, by hand; the tests use a stand-in for it. The agent's window, its panel item
    and the Agent page have not been looked at on a real desktop. TorqOS ships no AI program and no model:
    you install Claude Code and sign in to it yourself.

## Set it up

An administrator does this once for the machine, as recorded changes like any other. Claude Code is not
open-source software, and TorqOS installs nothing of that kind until you allow it:

```text
alice@torq# set torq unfree packages claude-code
alice@torq# set torq agent tools claude-code
alice@torq# set torq agent engine claude-code
alice@torq# commit
```

(or the Agent page of TorqOS Settings: steps 1 to 3, then *Your Changes*). The first line allows it, the
second installs it for the agents, the third makes it **the engine**: the program that answers when you
ask. Then sign in once, in a terminal:

```text
alice@torq> claude
```

and type `/login` (your Claude subscription, or an API key). The sign-in is kept in your agent's account,
never in yours. Starting a program as your agent is an administrator's command today: if you are not one,
ask one.

What your agent reads when it works (logs, files in the folder you shared, the configuration) goes to the
service of whoever makes the engine: for Claude Code, Anthropic. The Agent page and the agent's window say
so.

The system journal is off limits to the agents unless the owner sets `torq agent journal true`. Turning
that on gives every agent account, and so every program run as it, read access to every service's log, so
it is off by default.

## Ask your agent

```text
alice@torq> torq ask "why is my wifi flaky?"
alice@torq> torq ask "install htop for me"            # says "Proposed #12" if it proposes a change
alice@torq> torq ask --forget                         # deletes the conversation
alice@torq> request system agent forget               # the same, in torqcli (the Agent page's Forget)
```

The answer comes as it is written; what the agent does on the way is shown in plain words ("ran a
command", "read a file", "looked at the configuration"), and each change it proposes by its number.
**Ctrl-C** stops it. One question at a time. Your agent keeps your conversation until you delete it: a
stop, closing the window, or a restart of the machine continues it. Only you can reach your agent: its
socket is yours, and it answers no one else, root included.

## `claude`, in a terminal

Typing `claude` in a terminal starts Claude Code **as your agent**, with the folder you are in shared with
it. It sees that folder and its own account's home, and nothing else of yours. Whatever you type after
`claude` is passed on to it.

In your home folder itself, outside your home, or in a hidden folder of your home (`~/.ssh`, `~/.config`:
keys and settings live there) nothing is shared: it says so in one line and works in the agent's own
folder. The spelled-out form is `torq agent run --share <a folder of yours> -- claude`; `torq agent run`
starts any other program as your agent the same way. Turn the short form off with `set torq agent
plainCommand false`. A Claude Code you installed in your own account is found first, and runs as you,
outside the agent's sandbox.

Each time it starts, TorqOS links its own tools into Claude Code: a set named `torqos` (read the machine's
alarms, its record of changes, configuration, logs and hardware; propose a change) and a short guide to
how this machine is configured. Your other Claude Code settings are left as they were. Claude Code's own
"skip permissions" mode is safe to use here: there is nothing it can apply.

## Instructions for your agent

A text of your own that your agent is always given: how you like answers, what to explain, what never to
do. `torq agent instructions` shows it, `torq agent instructions edit` opens it in your editor, `torq
agent instructions clear` removes it; the Agent page has **Edit…**. It lives in your home
(`~/.config/torq/agent-instructions.md`, at most 16 KiB), where your agent cannot write, so nothing it
reads can change what you told it.

## On the desktop: the panel, its window, and the Agent page

**In the panel**, beside TorqOS's own mark, is your agent's: a speech bubble, quiet while it is idle,
in the accent colour while it works, in the warning colour when something went wrong. Click it: it
says how the agent stands (idle, working, not set up for you, not signed in, not reachable) and what
answers. Type a question and press Enter: the answer comes below as it is written. Each change it
proposes is listed with **Review** (TorqOS Settings, at Proposals); **Stop** ends an answer while it works,
wherever you asked it; **Open** opens its window. Looking does not start your agent; asking does.

**Its window**, *TorqOS Agent* in the menu, keeps the conversation of this sitting: each question, its
answer and what the agent did on the way. **Stop**, **Forget…** (it asks first: the conversation is then
deleted), and **Review** for each proposal. There is one window for you: opening it again brings back the
one that is open.

**The Agent page** of TorqOS Settings shows what your agent is: your agent account and whether its
service is on this machine, what answers and how to sign it in, your instructions for it with **Edit…**,
how long the conversation is with **Forget…**, and buttons to its window and to Proposals. Choosing the
engine is a change to the system like any other: the page's buttons put the line in your draft, and you
commit it from *Your Changes*, recorded, undoable, with your approval. (`set torq agent people …` adds a
person, except the first time: until the list is written its default is you, the owner, and the first `set`
replaces that, so name yourself too.)

**Decline.** Beside **Review**, wherever one of your agent's proposals waits for you (TorqOS Settings'
Proposals page, the agent's applet, its window), **Decline** turns it down: it shows you the proposal and
asks first, and a declined proposal is never applied (`request system decline <n>` in torqcli does the same,
also for other changes waiting for you, which may ask an administrator's password). A proposal whose apply
failed waits again, marked "its apply failed once, so apply it again or decline it".

### Ask from anywhere

- **Super+Shift+A** opens the agent's window (Super+A stays COSMIC's App Library; change either in COSMIC
  Settings → Keyboard → Shortcuts).
- **The launcher** (Super+/): type `ask why is my wifi slow?` and press Enter: the window opens with the
  question in its box, and you press Ask.
- **The terminal**: `ask` on its own, then type the question and press Enter, no quoting needed. With the
  question on the same line, quote it as you would any text the shell reads: `ask "why is $HOME full?"`.
- **Ask the agent** beside an alarm, under a command that failed, in a setting's side panel and on your
  uncommitted changes in TorqOS Settings; on an alarm in TorqOS's applet; on a tunnel that will not come up in
  the SSH tunnels applet.
- **A crash**: when one of your programs crashes, a notification offers **Ask the agent about this crash**.
  Nothing about the crash is read or sent unless you click it.

Whatever these attach (the alarm, the error, the crash's summary) is shown above the question before it is
sent, and you can remove it. You press Ask. It is text anyone could have put there, so your agent is given
it marked as something attached to read, not as instructions.

Every one of these is your own program: it talks only to your own agent, runs nothing as root, and
holds no secret.

## What your agent can reach, and the record of what it asked

Your agent's account reaches one thing on the network: a TorqOS service on your own machine, the **model
proxy**. Nothing else: no website, no other machine, no other program on this one. Your own account is not
affected at all.

- **Its model.** When your agent asks its model (Claude, for Claude Code), the request goes through the
  proxy, which passes it on unchanged, your sign-in included, and keeps none of your sign-in. It writes
  down what happened: when, which model, how many tokens, what was new in the conversation (your question,
  or what each tool returned) and what the model answered (its words, and each tool it called).
- **Sites.** A program run as your agent reaches a site only as HTTPS, through the proxy, and only if the
  owner allows that site:

    ```
    configure
    set torq agent network hosts github.com
    commit
    ```

    `*.github.com` allows every site under a name. Each site your agent asks for is written down, allowed or
    refused; what it then exchanges with an allowed site is encrypted and is not. A site you list is one your
    agent can read, and can send anything it can read to.

- **The whole web** is one setting, off by default: `set torq agent network web true`. Your agent can then
  read any website, and send anything it can read to any website; the record shows only which sites it
  reached. Claude Code's own web search works without it (the search runs at Anthropic); fetching a page
  you name does not.

**The record** is kept for you in `/var/lib/torq-model-proxy/<you>/`, a file for each day, one line for
each request. You can read it; no one else can but an administrator acting as root, and no agent can read
or change it. A command and a page that show it are not built yet. It holds what your agent read (the
contents of files, the output of commands), so it is as private as your files. It is kept to a quarter of a
gigabyte for each person: the oldest days go first.

Two things to know about Claude Code:

- Its **claude.ai connectors** (tools that run at Anthropic and act in your accounts, such as Gmail) do not
  work for your agent unless the owner lists `api.anthropic.com` and `mcp-proxy.anthropic.com`. Listing the
  first also lets a program run as your agent ask the model past the record.
- **Signing in** from inside the agent's account has not been tried since this was built. If `/login`
  does not finish, an administrator can allow `api.anthropic.com` while you sign in (`set torq agent network
  hosts api.anthropic.com`, `commit`) and take it off again after (`delete torq agent network hosts
  api.anthropic.com`).

Your agent cannot change any of these settings: no agent can propose them.

## Your agent's proposals always wait for you

Logs, device names, web pages, files and configuration values are text anyone could have written, and a
line in a log or on a page can carry instructions meant for an AI. TorqOS cannot see what your agent has
read. So it never lets your agent change anything by itself:

- everything your agent proposes waits until **you** have read it and approved it, one proposal at a time;
- there is no setting that lets it apply changes without asking, for a while or for a kind of change;
- when you apply one, TorqOS shows what the change does, as the system records it and not as the agent
  describes it, tells you that your agent proposed it and which of TorqOS's own tools it used to read things
  before proposing (for example "the system's log, the record of changes"; TorqOS does not see what else it
  read), and asks. To anyone else on the machine the same proposal reads "<your name>'s agent proposed this".

Judge the change you are shown, not the reason the agent gives for it.

Nothing but TorqOS's own module may change your agent's service. A TorqOS machine with an agent does not
build if a package brings a unit file or a drop-in for `torq-agent@<you>.service` or its socket (or one
that applies to every service, `service.d`): the build's log names the file and the package it comes
from. Nor does it build if another part of the machine's configuration changes one of those units (who it
runs as, what it runs, its sandbox, its socket): `commit check` and the build's log name the unit and the
setting. There is no setting that turns this off.

## Apply a proposal

```text
alice@torq> show system proposals
alice@torq> request system apply 42
```

`request system apply` shows what the change does and asks. A system change asks your password.

A change the agent proposes to the network or SSH, a service's unit or a file in `/etc`, the boot, the
hardware, the console or the display could cut you off. It is applied as a **commit confirmed**: it rolls
itself back in 10 minutes unless you confirm it (`configure`, then `commit`), so if it cuts you off, you
get your machine back by waiting.

## Decline a proposal

```text
alice@torq> request system decline 42
```

Not wanted? Decline it: it is never applied after, by anything, and it no longer waits in `show system
proposals`. Your agent's proposals, and anything wholly in your own folder, are yours to decline without a
password; anyone else's needs an administrator's password. The agent cannot decline for you. It stays in
the record of changes as declined; to have it after all, ask for it again. A proposal whose apply failed
still waits (marked so in `show system proposals`): apply it again, or decline it.

## When it uses a secret

Your agent can use a token or an SSH key that it never reads: TorqOS sends the request or signs for it
(see [Secrets](../reference/security.md#secrets-for-the-agent-and-programs)). You say which secrets it
may use.

When your agent uses one, you are asked: the use waits two minutes for you, and only for you. On the
desktop a notification says "Your agent asks to use a secret"; click it, or click TorqOS in the panel, and
answer there: **Allow once**, **Allow for…**, **Deny**, or (for a use that belongs to a session) **Deny,
and don't ask again this session**. The panel shows what TorqOS knows first (which secret, which host),
and the agent's own words below, marked as the agent's. At a terminal, `show system secrets waiting`
lists what waits, and `request system secret approve <n>` or `deny <n>` answers it. With no answer the use
is refused.

Your agent asks every time, because TorqOS cannot see what it has read (a web page or a log line can carry
instructions meant for it). To let it use one secret without asking, for a while or until you say otherwise, see
[Letting your agent use a secret without asking](../reference/security.md#letting-your-agent-use-a-secret-without-asking):
it asks your password, and that page says what you are giving.

The panel's dialog has not been tried on a real machine yet.

## If you used an earlier version

TorqOS once had a small agent of its own (`torq doctor`, `torq agent propose`, the settings under `torq
agent doctor` and `torq agent model`, and "trusting" the agent for a while with `request system agent
trust`). All of that is gone. `torq doctor` and `torq agent propose` now say so in one line: ask your
agent instead, with `torq ask`.

If your machine's configuration still has one of those settings, the update is refused, and the refusal
names each one that is set, the file it is in, and the line that deletes it. Delete what it names, in the
same commit as the update:

```text
alice@torq> request system update
alice@torq> configure
alice@torq# delete torq agent doctor
alice@torq# commit
```

(`delete torq agent model` the same way, if the refusal names it; delete only what it names.) The same
lines work once the machine already runs the new TorqOS, for example when its configuration came from a
repository that still had the setting.

If you had let the agents read the system journal (`torq agent doctor journal true`), that setting is now
`torq agent journal`. The TorqOS you are updating from does not know it yet, so it cannot be set in the
same commit: after the update is installed, make a second one.

```text
alice@torq> configure
alice@torq# set torq agent journal true
alice@torq# commit
```

Between the two commits your agents do not read the journal.
