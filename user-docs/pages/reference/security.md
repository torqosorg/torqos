# Security defaults

## Out of the box

| | Default |
|---|---|
| Network | **Nothing listens** until you ask for it: no SSH, and no link-local name answering (LLMNR, mDNS) |
| `root` | **Locked**, no password, never logs in over SSH |
| Accounts | The owner's account is created at first boot; there is no default password |
| Passwords | Kept outside the configuration, readable by the system only, so their hashes are never in its history |
| SSH (once on) | Keys only unless passwords are allowed; 3 tries per connection; the owner only; repeated wrong guesses blocked |
| Releases | Images and updates are signed; every update's signature is checked before it is loaded |
| History | A hash-chained ledger of every change, checked at boot and every 15 minutes. Against someone with root on the same machine it holds only if the configuration is also pushed to a remote, which is off by default |
| Footprint | The base has no desktop and no graphical stack, and no installer tools on an installed machine. A desktop (COSMIC) is an opt-in configuration: see [The desktop](../guide/desktop.md) |

## How changes are controlled

- **One writer.** Only TorqOS's broker writes the machine's configuration. The configuration tree refuses a list of settings that would change the broker itself, but plain Nix files attached to the tree, and other settings that run code as root, can still change it: what stands there is an administrator reading and approving the change. A stray build of
  `/etc/nixos` (where stock NixOS keeps its configuration) is refused.
- **A person approves.** A system change shows its comparison and needs a yes at a terminal (on a desktop, in a password dialog).
- **Nothing hidden.** Any Nix file in a change is shown as its raw diff before you approve it.
- **Drift is visible.** Changes made around TorqOS (with `sudo`, for example) are reported by
  `show system alarms`, and `request system repair` puts the machine back.
- **The broker is Rust** (about 9,000 lines, with `unsafe` and dead code forbidden by the compiler). Other things also run as root: a few shell scripts, and `torqcli` itself at setup and install. The broker's own systemd units are not sandboxed yet.

## Root and sudo

The owner can use `sudo`, as on any Linux, and a root shell can go around TorqOS. TorqOS does not try
to forbid that; it makes it visible instead, through the alarms and the ledger's hash chain.

**Strict mode** is the stronger choice, off by default: `set torq security strict enable true`. Then
`sudo` runs nothing you could get a shell from, and the one way to a root shell is
`torq break-glass "why"`. It asks your password every time, writes who and why into the ledger, sends
that to your remote if you have one, and only then opens the shell. Daily work needs no `sudo`: `torqcli`
does not use it.

Before turning it on, know what it costs. If the ledger cannot be written, on a full disk for example,
there is no shell and no fallback: the way back is the machine's console, starting an older
generation from before strict mode if one is left, and otherwise booting from other media. No account
may be in a group that amounts to root, such as `docker`; the commit is refused until it is out.

## Your data, before each change

A newly installed machine keeps a read-only snapshot of `/home` and `/var/lib` before each
system change, the newest ten. `show system snapshots` lists them, and `request system snapshot restore <n>
<path>` puts a path back as it was before change `n`. What is there now is set aside beside it, not
lost. It asks an administrator's password and goes in the ledger. Stop a service before restoring its
data, and start it again after.

Something you delete stays in the snapshots until they age out, ten system changes later. They hold your
files, not a backup of them: a failed disk takes them too. Machines installed on ext4 have none.

To change how they work: `set torq snapshots keep 20`, or `set torq snapshots enable false`, in configuration
mode. `request system snapshot take` takes one now, before work you may think better of, and
`request system snapshot delete <n>` removes one, for the room or for what is in it: that goes in the ledger
first, and there is no undoing it.

## Your data, somewhere else

Snapshots live on the same disk. A backup is NixOS's own restic, set in configuration mode like
any setting, to a disk, a NAS, SFTP or S3:

```
set services restic backups nas repository sftp:backup@nas:/torq
commit
```

It holds `/home` and `/var/lib` unless you set `paths`, and never `/var/lib/torq/secrets` (your
password hashes, and the backup's own key) unless `paths` names it. It runs daily, encrypted.

Its key, the repository's passphrase, is never in the configuration: `request system backup key nas`
asks for it here, twice, then your password, and keeps it where only the system can read it. The
ledger says it was given, never what it is. Keep it somewhere else too: without it, nothing in the
backup can be read. A remote target's own login, an SSH key or S3's keys, you place yourself.

`show system backups` lists each backup's snapshots and how it last ran. `request system restore nas
/home/you/docs` puts a path back from the newest snapshot, or from one you name:
`request system restore nas 4bd1d0c5 /home/you/docs`. As with a snapshot, it is written as the owner
of the folder, what is there now is set aside beside it, it asks an administrator's password, and it
goes in the ledger.

## Secrets for the agent and programs

A secret is a token or key that a program, such as the agent, may use and never read. TorqOS keeps it
encrypted and outside the configuration; the ledger records each change and first use, never the
value. It is for that and nothing more.

Secrets are managed from the command line (`request system secret …`). For your everyday passwords, use a password manager such as 1Password: TorqOS does
not try to be one.

After 0.4.0-rc1, TorqOS Settings has a **Secrets** page that does what those commands do, and shows
each command beside its button:

- It lists the secrets `show system secrets` lists, one on each row: its name, whose it is, whether it
  has a value, and what it may be used for. It never shows a value. You choose a row, or "A new
  secret" and type a name, and the rest of the page is about that secret.
- For the secret you chose, it shows what is allowed now, and allows or takes back a use (a host over
  HTTPS or SSH, or the agent) and sets when a person is asked before the agent uses it. A secret of
  your own needs no password; a system secret needs an administrator, who approves in COSMIC's
  dialog, as for any system change.
- It shows what your agent may use without asking for now (below), with a button that ends each, and
  marks in red a secret that your agent uses without ever asking you.
- It deletes a secret, after asking once more: a deleted value cannot be brought back.
- It takes a value, typed twice in fields that show dots. The value goes to the broker the way
  `torqcli` sends it. It is not part of any command, log or history entry.

What the page does not do:

- **It does not show a value back.** `request system secret show <name>`, in a terminal, does. Typed
  on the Command Line page of TorqOS Settings, that command and `request system secret set <name>`
  open a terminal.
- **It does not read a value from a file.** For an SSH key (several lines), use
  `request system secret set <name> file <path>` in a terminal.
- **It does not clear your clipboard.** A value you paste into the page stays in the clipboard, where
  other programs of yours can read it, until you copy something else.
- **It is not where you answer your agent.** When your agent asks to use a secret, you answer in the
  panel or at a terminal (next).

### When your agent asks to use a secret

You decide whether your agent may use a secret at all (`request system secret allow <name> agent …`).
When it does, you are asked. TorqOS cannot see what your agent has read (a web page or a log line can
carry instructions meant for it), so it asks you each time unless you said otherwise for this secret
([below](#letting-your-agent-use-a-secret-without-asking)).

A use that asks waits two minutes for you. Only you are asked: the person whose agent it is, also for
a system secret. With no answer, the use is refused. The agent never gets the value either way.

**On the desktop**, a notification says "Your agent asks to use a secret". Click it, or click TorqOS
in the panel, whose mark has a badge while a use waits. The panel shows:

- First, what TorqOS knows for itself: the secret and whose it is, whether it is for an HTTPS request
  or an SSH sign-in and to which host, why you are asked, and how long you have left.
- Then, under a line that says so, what your agent says about it. These are the agent's own words.
  Nothing has checked them, and text the agent has read can put words in its mouth. Decide by the
  lines above them.

The buttons:

- **Allow once**: this one use goes ahead.
- **Allow for…**: this use goes ahead, and so do later uses of this one secret, without asking you,
  for 60 minutes or for 480 (8 hours). It asks your password first. If you cancel the password,
  nothing is allowed and the use still waits. If you already let your agent use this secret for a
  while, the panel says so: choosing a time then asks your password and puts the new time in place of
  the old. If you cancel the password at that point, the old one stays.
- **Deny**: this use is refused.
- **Deny, and don't ask again this session**: this use is refused, and so is every later use of that
  secret in that session of the agent's, at once. A new session can ask again. This button is there
  only when the use belongs to a session; a use that names none has only **Deny**.

No button is highlighted, and no key answers for you: you choose one.

One use is shown at a time, and the panel says how many more are waiting. When what the panel shows
changes (another use arrives, or the next one takes its place), its buttons take no press for one
second, so a click meant for what was there before does nothing. After you answer, the panel says
what was done, with an **OK**; if a use is answered at a terminal, or its two minutes run out, it
says that instead. If something was not done, it says so in red and keeps saying it until you press
**OK**: read it, because part of what you asked for may have happened. For example, if a use ran
out of time while you typed your password, what you allowed still holds, though that use was refused.

The panel's dialog is new and has not been tried on a real machine yet. If it does not appear, answer
at a terminal.

**At a terminal:**

```text
alice@torq> show system secrets waiting
alice@torq> request system secret approve 7
alice@torq> request system secret deny 7
alice@torq> request system secret deny 7 session
```

`show system secrets waiting` lists each use with the same facts, then the agent's own words, marked
as the agent's, and until when it waits, in local time, with the time left. `deny 7 session` is the "don't ask again this
session" answer.

Each answer is recorded in the ledger: who answered, yes or no, and whether in the panel or at a
terminal. So is a use that ran out of time. The value is never recorded, and neither are the agent's
words.

### Letting your agent use a secret without asking

You can let your own agent use one secret without asking you. There are two ways, each for one named
secret, and each asks your password and is recorded:

| What | Command | How long |
|---|---|---|
| One secret, for a time | `request system secret allow <name> for <minutes>` | 1 to 480 minutes |
| One secret of your own, until you say otherwise | `request system secret approve <name> without-asking user <you>` | until you choose `each-use` again |

For the first, add `user <you>` for a secret of your own; without it, the command means a system
secret that your agent is allowed. In the panel, **Allow for…** gives it for the secret being asked
about, for 60 or 480 minutes, and so does the Secrets page of TorqOS Settings. The second is only for
a secret of your own: an administrator cannot say it for a system secret. To take it back:
`request system secret approve <name> each-use user <you>`.

Think before using either. Text your agent has read (a web page, a file, an issue someone wrote) can
then make it use that secret without you being asked. What still holds: the secret goes only to the
hosts its rule names, the agent never reads it, and neither way lets the agent use a secret its rule
does not allow or change the system. So give such a secret as little power as you can at those hosts:
a fine-grained token, a deploy key.

`show system secrets` marks a secret that never asks with `[!! NEVER ASKS YOU]`.

To see what is allowed now, and to end it:

```text
alice@torq> show system secrets allowed
alice@torq> request system secret allow revoke gh user alice
alice@torq> request system secret allow revoke ci
alice@torq> request system secret allow revoke all
```

Ending needs no password. `allow revoke <name> user <you>` ends what you allowed for your own secret
of that name; without `user <you>` it means the system's secret of that name, as when you allowed
it. `allow revoke all` (or just `allow revoke`) ends everything you allowed. Times in these lists are
your machine's local time.

The Secrets page of TorqOS Settings shows everything you allowed at the top, with a button that ends
each one. Under a secret you choose it also lists anyone else's agent that is allowed it for now;
that is theirs to end.

#### If you set this up before 2026-11-16

Earlier builds after 0.4.0-rc1 had more words here, and most of them did nothing. What you will
notice:

- **A rule that said `first-use` or `never`** asked you at every use, and still does: it now reads as
  `each-use`. A rule that said `never-even-tainted` did not ask, and still does not: it reads as
  `without-asking`. Your rules file keeps the old word until `torqcli` next changes that file (any
  `request system secret allow`, `disallow`, `approve` or `delete` of yours); that change then writes
  the whole file in today's words, so its record shows the old words replaced beside what you asked
  for. Nothing changes in what you are asked.
- **`allow <name> session` and `allow all for <minutes>`** let no use through. `allow <name> session`
  is no command now, and `allow all for <minutes>` now means a secret that is itself named `all`. If
  you had allowed something either way, it no longer shows in `show system secrets allowed`.
- **`allow <name> for <minutes> tainted-too`** is `allow <name> for <minutes>`. What you allowed that
  way holds until its time.
- **`allow all revoke`** is gone with what it ended. `allow revoke all` ends everything you allowed.

## Checking a release yourself

See [Install → Check the download](../start/install.md#2-check-the-download). You need only OpenSSH
and coreutils.

## Reporting a vulnerability

Please report a suspected vulnerability privately to the maintainers rather than in a public issue.
