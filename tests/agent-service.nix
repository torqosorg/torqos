# NixOS VM test: each person's agent as a service (ADR-051 3, ADR-055 3) — `torq-agent@<person>.service` on
# its socket, the bridge to the engine, with `torq ask` its terminal client and `torq agent run` beside it.
# alice and bob each have an agent; the engine is Claude Code, standing in (tests/stand-in-engine.nix: the real
# one is unfree and needs a sign-in). Each claim fails if
# its guard is taken out:
# - `torq agent engine` is in the configuration tree, and the service is there because an engine is chosen;
# - each person's socket is theirs alone; bob cannot connect to alice's, and the service itself refuses anyone
#   but alice — root, or bob through a socket alice opened to all;
# - the service and the engine run as alice's agent account, in the launcher's sandbox and more: from inside,
#   the engine sees its own home and no other, no runtime directory, no other agent's files, and has no
#   privilege; the system is read-only to it;
# - not signed in is said with what to do; signed in (under `torq agent run`, as the person would), it answers;
# - the answer is passed on, each tool in plain words; the conversation is the engine's, kept across a stop
#   and a restart of the service, and ended by forget;
# - what it proposes through TorqOS's tools is told by its number, and is the broker's, as alice's agent's,
#   an agent's: it waits for alice;
# - Ctrl-C stops the answer, all of it (a program in a session of its own, one out of sight), and the service
#   stays; an engine that dies is said;
# - a person's standing instructions (their own file) are given to the engine, in the service and by `claude`
#   typed plainly; a changed text restarts the engine in the same conversation; forget deletes the engine's
#   own record of the conversation;
# - `claude` typed plainly runs Claude Code as the person's agent in the folder they are in, and with nothing
#   shared where that folder is never shared; someone who is not an administrator is told so;
# - sharing a folder works while the service and its engine run, both frozen through the walk and thawed after
#   (a walk that fails, or is killed, included); `torq agent run` works beside a running service; a
#   `torq agent run` session already running still makes a share refuse.
#
# torqcli's own commit as the agent runs for real, on a real configuration, in tests/agent-check.nix (ADR-056).
# This VM's config repo is a stub: a stand-in `torqcli` makes the proposal the commit would, over the broker's socket as the agent, naming
# torq-mcp's session; everything else torqcli is asked, the real one does.
{ pkgs, self }:

let
  hash = "$6$torqsocketsalt$Z7RLey3l5vKGfukhfuZYYM/EjWNo7QM8.65r88XmZ9z3/jBCY.NAnH7RwYpiLoE2fedxUd/I7UFcKDX1KEXDg/";
  claude = import ./stand-in-engine.nix { inherit pkgs; };
  # A program that makes itself undumpable (prctl PR_SET_DUMPABLE 0: its /proc files then root's), then
  # waits <seconds>.
  undumpable = pkgs.writeScript "undumpable" ''
    #!${pkgs.python3}/bin/python3
    import ctypes, sys, time
    assert ctypes.CDLL(None).prctl(4, 0, 0, 0, 0) == 0
    time.sleep(int(sys.argv[1]))
  '';
  # torqcli's commit as the agent, standing in (see above): the proposal it would make — htop in alice's
  # packages — over the broker's socket, as the agent, naming its session. Anything else: the real torqcli.
  torqcli = pkgs.writeShellScriptBin "torqcli" ''
    case "''${1-}:''${2-}" in
      -c:*"commit comment"*)
        base=$(${pkgs.git}/bin/git -c safe.directory='*' -C /var/lib/torq/config rev-parse HEAD)
        diff=$(printf -- '--- /dev/null\n+++ b/users/alice/packages.json\n@@ -0,0 +1 @@\n+["htop"]\n')
        ${pkgs.jq}/bin/jq -cn --arg d "$diff" --arg b "$base" --arg s "$TORQ_AGENT_SESSION" \
          '{args: ["--json", "propose", "--base=\($b)", "--reason=htop for alice", "--session=\($s)"], diff: ($d + "\n")}' \
          | ${pkgs.socat}/bin/socat -t 30 - UNIX-CONNECT:/run/torq/broker.sock
        ;;
      *) exec /run/current-system/sw/bin/torq "$@" ;;
    esac
  '';
  # setfacl, noting the agent service's freezer state each time it is run (the walk's ACLs): the walk seen.
  setfacl = pkgs.writeShellScriptBin "setfacl" ''
    systemctl show -P FreezerState torq-agent@alice.service >> /tmp/freezer-seen
    if [ -e /tmp/fail-setfacl ]; then exit 1; fi
    if [ -e /tmp/hang-setfacl ]; then sleep 600; fi
    exec ${pkgs.acl}/bin/setfacl "$@"
  '';
in
pkgs.testers.runNixOSTest {
  name = "torq-agent-service";
  # `torq unfree` sets nixpkgs.config (modules/unfree.nix), so the node builds its own package set.
  node.pkgsReadOnly = false;

  nodes.machine = { pkgs, lib, ... }: {
    imports = [ self.nixosModules.default ];
    torq.user = "alice";
    torq.desktop.enable = false;
    users.users.root.hashedPasswordFile = lib.mkForce null;
    users.users.alice.hashedPasswordFile = lib.mkForce null;
    users.users.alice.hashedPassword = hash;
    users.users.bob = { isNormalUser = true; hashedPasswordFile = lib.mkForce null; hashedPassword = hash; };
    # carol has no agent, and a Claude Code of her own in her own profile (a stand-in that says so).
    users.users.carol = { isNormalUser = true; hashedPasswordFile = lib.mkForce null; hashedPassword = hash; packages = [ (pkgs.writeShellScriptBin "claude" "echo \"carol's own, as $(id -un)\"") ]; };
    torq.agent.people = [ "alice" "bob" ];
    torq.broker.rebuildCommand = "true";
    # The check of a proposal (ADR-056) puts a copy of the config repo in the store: with the little room a
    # test VM's store has (under half a gigabyte to write in, less than `min-free`), Nix would collect garbage
    # first, each time: 40 seconds, measured.
    nix.settings.min-free = 0;
    # The engine: Claude Code chosen, allowed, installed for the agents, and standing in.
    torq.unfree.packages = [ "claude-code" ];
    torq.agent.tools = [ "claude-code" ];
    torq.agent.engine = "claude-code";
    torq.agent.claudeCode = claude;
    environment.systemPackages = [ pkgs.socat pkgs.jq pkgs.tmux pkgs.procps pkgs.strace (lib.hiPrio torqcli) (lib.hiPrio setfacl) pkgs.nftables ];
    # avahi, as a desktop runs it (modules/cosmic/cosmic.nix): what it answers anyone, it must never answer an agent.
    services.avahi.enable = true;
  };

  testScript = ''
    import json, re, shlex
    machine.wait_for_unit("multi-user.target")
    machine.wait_for_unit("torq-broker.socket")
    repo = "/var/lib/torq/config"
    machine.succeed(f"mkdir -p {repo} && echo '{{}}' > {repo}/desktop.json && touch {repo}/flake.nix && torq-broker init")
    unit = "torq-agent@alice.service"
    sock = "/run/torq-agent/alice.sock"
    home = "/var/lib/torq-agent-alice"
    mem = "/var/lib/torq-agent-memory/alice"
    engine = "pgrep -u torq-agent-alice -f 'bin/claude -p --input-format stream-json'"

    def as_user(user, cmd, ok=True):
        code, out = machine.execute(f"su - {user} -c {shlex.quote(cmd)}")
        assert (code == 0) == ok, f"{user}: {cmd}: exit {code}:\n{out}"
        return out

    def raw(user, request, path=sock):
        cmd = f"printf '%s\\n' {shlex.quote(json.dumps(request))} | socat -t 10 - UNIX-CONNECT:{path}"
        # (A refused connection is closed with its request unread: socat then says "reset", and fails.)
        return [json.loads(l) for l in machine.execute(f"su -s /bin/sh {user} -c {shlex.quote(cmd)}")[1].splitlines()]

    def status(running, turns, conversation):
        return [{"type": "status", "running": running, "turns": turns, "conversation": conversation}]

    def entries():
        return [json.loads(l) for l in machine.succeed("torq-broker log --json --brief").splitlines()]

    def prop(name):
        return machine.succeed(f"systemctl show -P {name} {unit}").strip()

    def starts(agent_home=home):
        return [json.loads(l) for l in machine.succeed(f"cat {agent_home}/.stand-in/starts").splitlines()]

    with subtest("the engine is a setting of the tree, and nothing of TorqOS's own old loop is here"):
        schema = json.loads(machine.succeed("cat /etc/torq/nixos-schema.json"))
        found = [e for e in schema["settings"] if e.get("loc") == ["torq", "agent", "engine"]]
        assert len(found) == 1 and found[0]["type"]["name"] == "nullOr" and found[0]["type"]["elem"]["values"] == ["claude-code"], found
        assert not [e for e in schema["settings"] if e.get("loc") == ["torq", "agent", "service", "enable"]], "the old switch is gone"
        assert json.loads(machine.succeed("cat /etc/torq/agent-tools.json")) == {"tools": ["claude-code"], "engine": "claude-code", "plainCommand": True}
        # Removed with the old loop (ADR-055, phase 2): its settings, its program, its second account.
        gone = [e["loc"] for e in schema["settings"] if e.get("loc", [])[:3] in (["torq", "agent", "doctor"], ["torq", "agent", "model"])]
        assert gone == [], gone
        assert [e for e in schema["settings"] if e.get("loc") == ["torq", "agent", "journal"]], "the journal's setting is the agents' own now"
        assert json.loads(machine.succeed("cat /etc/torq/agent.json")) == {"journal": False}
        machine.fail("test -e /run/current-system/sw/bin/torq-doctor")
        machine.fail("systemctl cat llama-cpp.service")
        machine.fail("getent passwd torq-resident-alice")

    with subtest("each person's socket, theirs alone; the service started by it, not before"):
        machine.wait_for_unit("torq-agent@alice.socket")
        machine.wait_for_unit("torq-agent@bob.socket")
        assert machine.succeed(f"stat -c %a:%U {sock}").strip() == "600:alice"
        assert machine.succeed("stat -c %a:%U /run/torq-agent").strip() == "755:root"
        assert prop("ActiveState") == "inactive", prop("ActiveState")

    with subtest("its sandbox: alice's agent, no privilege, every home and runtime directory out of reach"):
        want = {
            "User": "torq-agent-alice", "Group": "torq-agent-alice", "ProtectHome": "yes", "PrivateTmp": "yes",
            "NoNewPrivileges": "yes", "CapabilityBoundingSet": "", "AmbientCapabilities": "", "ProtectSystem": "strict",
            "LockPersonality": "yes", "RestrictNamespaces": "yes", "ProtectProc": "invisible", "PrivateIPC": "yes",
            "PrivateUsers": "no", "RestrictAddressFamilies": "AF_INET AF_INET6 AF_NETLINK AF_UNIX", "StandardInput": "socket",
            "WorkingDirectory": "/var/lib/torq-agent-memory/alice", "Delegate": "no",
        }
        got = {k: prop(k) for k in want}
        assert got == want, got
        text = machine.succeed(f"systemctl cat {unit}")
        assert "@system-service" in text, "its system calls filtered"
        calls = prop("SystemCallFilter")
        assert "socket" in calls.split() and not any(c.startswith("io_uring") for c in calls.split()), "no io_uring (a socket of any family past RestrictAddressFamilies)"
        assert f"ReadWritePaths={home}\n" in text and text.count("ReadWritePaths=") == 1, "its own home, and nothing else, to write"
        assert "InaccessiblePaths=-/run/credentials\n" in text and "Credential=" not in text, text
        # Through torq-agent-scope: the socket families' filter, then Landlock's scopes, then the bridge.
        assert re.search(r"^ExecStart=/nix/store/\S+/bin/torq-agent-scope --families /nix/store/\S+-torq-agent-socket-families.bpf -- /nix/store/\S+/bin/torq-agent-service alice claude-code$", text, re.M), text
        assert "landlock_restrict_self" in calls.split() and "seccomp" in calls.split(), calls
        sec = machine.succeed(f"systemd-analyze security --no-pager {unit}")
        print(sec)
        found = re.search(r"Overall exposure level for \S+: ([0-9.]+)", sec)
        assert found, sec
        score = float(found.group(1))
        print(f"systemd-analyze security {unit}: {score}")
        assert score <= 2.0, score

    with subtest("not signed in: said, with what to do; signed in as the person would, it answers"):
        code, out = machine.execute("su - alice -c 'torq ask hello' 2>&1")
        assert code != 0 and "Claude Code is not signed in for your agent. Sign it in once, in a terminal: type `claude`, then type /login" in out, out
        assert "(Starting it is an administrator's command: if you are not one, ask one.)" in out, out
        assert "Not logged in" not in out and "Hello" not in out, out
        # The engine that was not signed in is ended: the next question is asked of one that sees the sign-in.
        machine.wait_until_fails(engine, timeout=30)
        assert raw("alice", {"type": "status"}) == status(False, 0, 1)
        # As the message says: `claude`, typed at a terminal (here in her home: nothing is shared, and it runs as her
        # agent all the same), then the stand-in's own word for /login.
        assert as_user("alice", "claude --stand-in-login").strip() == "signed in"
        assert machine.succeed(f"stat -c %U {home}/.stand-in/signed-in").strip() == "torq-agent-alice"

    with subtest("torq ask: alice's agent answers her; the service and the engine run as her agent"):
        out = as_user("alice", "torq ask hello")
        assert out.strip() == "Hello alice.", out
        machine.wait_until_succeeds(f"systemctl is-active {unit}")
        pid = prop("MainPID")
        assert machine.succeed(f"ps -o user= -p {pid}").strip() == "torq-agent-alice"
        # The engine: one process, the service's child, the agent's; started as the real one is, for a new
        # conversation, with TorqOS linked into its settings before it (the launcher's link, made by the service).
        epid = machine.succeed(engine).split()
        assert len(epid) == 1 and machine.succeed(f"ps -o ppid= -p {epid[0]}").strip() == pid, epid
        last = starts()[-1]
        assert last["args"] == ["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--permission-mode", "bypassPermissions"], last
        assert last["user"] == "torq-agent-alice" and last["cwd"] == home, last
        # Its environment is the service's making, the launcher's names and no others.
        # Its environment is made by the service from nothing: these, and what it is told of TorqOS's model proxy (ADR-057).
        assert last["env"] == ["ANTHROPIC_BASE_URL", "HOME", "HTTPS_PROXY", "LANG", "LOGNAME", "NO_PROXY", "PATH", "SHELL", "TERM", "USER", "https_proxy", "no_proxy"], last["env"]
        linked = json.loads(machine.succeed(f"cat {home}/.claude.json"))["mcpServers"]["torqos"]
        assert linked == {"type": "stdio", "command": "/run/current-system/sw/bin/torq-mcp", "args": [], "env": {}}, linked
        assert machine.succeed(f"readlink {home}/.claude/skills/torqos").strip() == "/etc/torq/skills/torqos"
        assert machine.succeed(f"stat -c %U {home}/.claude.json").strip() == "torq-agent-alice"

    with subtest("the conversation is the engine's, one process for it; what the service keeps is its name and a count, out of reach of torq agent run; deleted by forget"):
        out = as_user("alice", "torq ask 'remember?'")
        assert out.strip() == "You said hello.", out
        assert machine.succeed(engine).split() == epid, "the same process: one conversation"
        assert machine.succeed(f"stat -c %a:%U {mem} {mem}/conversation.json").split() == ["700:torq-agent-alice", "600:torq-agent-alice"]
        kept = json.loads(machine.succeed(f"cat {mem}/conversation.json"))
        # Its name at the engine, its count, and a mark of the instructions it began with (none): nothing of what was said.
        assert sorted(kept) == ["began", "questions", "session"] and kept["began"] is None and kept["questions"] == 2 and re.fullmatch(r"[0-9a-f-]{36}", kept["session"]), kept
        assert "hello" not in machine.succeed(f"cat {mem}/conversation.json")
        # A program run as the agent (torq agent run) does not see it.
        as_user("alice", f"torq agent run -- cat {mem}/conversation.json", ok=False)
        assert as_user("alice", "torq agent run -- ls -A /var/lib/torq-agent-memory").strip() == ""
        assert raw("alice", {"type": "status"}) == status(False, 2, 1)
        # The engine's own record of the conversation, where it keeps it in the agent's home, with what was said.
        record = machine.succeed(f"find {home}/.claude/projects -name {kept['session']}.jsonl").strip()
        assert record.startswith(f"{home}/.claude/projects/") and "hello" in machine.succeed(f"cat {record}"), record
        out = as_user("alice", "torq ask --forget")
        assert out.strip() == "The conversation is deleted. Your next question starts a new one.", out
        machine.fail(f"test -e {mem}/conversation.json")
        # Forget is true: the engine's record is deleted too, and no other conversation's is left of this agent.
        machine.fail(f"test -e {record}")
        assert machine.succeed(f"find {home}/.claude/projects -name '*.jsonl'").strip() == "", "a conversation's record is left after forget"
        # Forget ends the engine's process; the next question starts another, with nothing to continue.
        machine.fail(engine)
        assert as_user("alice", "torq ask 'remember?'").strip() == "I remember nothing."
        assert "--resume" not in starts()[-1]["args"], starts()[-1]
        # Another conversation now, by its number (what an open window resets on); the number says nothing else.
        assert raw("alice", {"type": "status"}) == status(False, 1, 2)
        assert machine.succeed(f"cat {mem}/generation").strip() == "2"
        as_user("alice", "torq ask hello")

    with subtest("what the engine sees and may do, from inside: its own home and nothing of anyone else's, no privilege"):
        machine.succeed("chmod 755 /home/alice && echo open > /home/alice/open && chmod 644 /home/alice/open && echo outside > /tmp/outside && chmod 644 /tmp/outside")
        machine.succeed("install -d -m 1777 /srv/open")
        machine.succeed("echo left-by-another > /dev/shm/outside && chmod 644 /dev/shm/outside")
        # The control: the agent account, outside the service, reads both, and writes where anyone may.
        machine.succeed("setpriv --reuid torq-agent-alice --regid torq-agent-alice --clear-groups cat /home/alice/open /tmp/outside /dev/shm/outside")
        machine.succeed("setpriv --reuid torq-agent-alice --regid torq-agent-alice --clear-groups sh -c 'touch /srv/open/x && rm /srv/open/x'")
        # What it must not reach though it shares the machine's network namespace (the security reader, 2026-11-17):
        # an abstract unix socket of root's, a process of its own account outside the service, a pair of TIPC
        # sockets (systemd's RestrictAddressFamilies judges `socket`, not `socketpair`), and avahi. Each the control
        # first: reached by alice herself, and the signal by the agent's account outside the service.
        machine.succeed("modprobe tipc")
        machine.succeed("systemd-run --unit=torq-test-abstract socat ABSTRACT-LISTEN:torq-test-root,fork SYSTEM:true")
        machine.wait_until_succeeds("echo | socat - ABSTRACT-CONNECT:torq-test-root")
        as_user("alice", "echo | socat - ABSTRACT-CONNECT:torq-test-root")
        machine.succeed("systemd-run --unit=torq-test-outsider -p User=torq-agent-alice sleep 900")
        outsider = machine.wait_until_succeeds("systemctl show -P MainPID torq-test-outsider | grep -v '^0$'").strip()
        machine.succeed(f"echo {outsider} > /srv/open/outsider")
        machine.succeed(f"setpriv --reuid torq-agent-alice --regid torq-agent-alice --clear-groups kill -0 {outsider}")
        pair = "import socket; a, b = socket.socketpair(socket.AF_TIPC, socket.SOCK_SEQPACKET); print('made')"
        assert "made" in as_user("alice", f"${pkgs.python3}/bin/python3 -c {shlex.quote(pair)}")
        machine.wait_for_unit("avahi-daemon.service")
        as_user("alice", "busctl call org.freedesktop.Avahi / org.freedesktop.Avahi.Server GetVersionString")
        machine.succeed("test -S /run/avahi-daemon/socket")
        out = as_user("alice", "torq ask look 2>/tmp/err")
        seen = json.loads(out)
        print(json.dumps(seen, indent=1))
        assert "agent: ran a command" in machine.succeed("cat /tmp/err"), "its step, in plain words"
        assert seen["user"] == "torq-agent-alice" and seen["home"] == home and seen["cwd"] == home, seen
        assert seen["uid"] == int(machine.succeed("id -u torq-agent-alice")), seen
        # Every home, root's, and the people's runtime directories: not there for it.
        for hidden in ["/home", "/run/user", "/root", "/run/credentials"]:
            assert seen[hidden] in ("PermissionError", "FileNotFoundError", []), (hidden, seen[hidden])
        assert seen["/home/alice/open"] in ("PermissionError", "FileNotFoundError"), seen
        # /tmp its own; another person's agent's home closed; the system read-only but its own home.
        assert seen["/tmp/outside"] == "FileNotFoundError", seen
        assert seen["bobs"] == "PermissionError", seen
        assert seen["write home"] == "written" and seen["write /etc"] in ("OSError", "PermissionError"), seen
        # Even where anyone may write, the service's view of the system is read-only (EROFS, not a refusal of the account).
        assert seen["write /srv/open"] == "OSError", seen
        # No capability, and none to gain.
        assert seen["caps"] == ["0000000000000000"] * 3 and seen["no new privs"] == ["1"], seen
        # /dev/shm is the unit's own: what another account left in the machine's is not there (the control,
        # above: the same account outside the service reads it), and what the engine writes there is.
        assert seen["/dev/shm/outside"] == "FileNotFoundError" and seen["/dev/shm"] == [] and seen["write /dev/shm"] == "written", seen
        assert "TemporaryFileSystem=/dev/shm:mode=1777,nosuid,nodev\n" in machine.succeed(f"systemctl cat {unit}")
        # What is so, recorded: the engine has the service's uid, and can write where the service keeps the
        # conversation's name (so the service trusts nothing it finds there: agent/src/service/kept.rs).
        assert seen["write state"] == "written", seen
        # Its PATH: its own profile, then the system's; none of the set-id wrappers.
        assert seen["path"] == "/etc/profiles/per-user/torq-agent-alice/bin:/run/current-system/sw/bin", seen
        # It cannot trace the bridge that started it, nor open its memory (the same uid, not its descendant:
        # refused, or not there for it at all); a child of its own, it can (the control for the memory).
        assert seen["trace parent"] in ("errno 1", "errno 3"), seen
        assert seen["parent's memory"] in ("PermissionError", "FileNotFoundError"), seen
        assert seen["child's memory"] == "opened", seen
        # Landlock's scopes: root's abstract socket refused (EPERM), one of its own reached; no signal to its own
        # account outside the service (EPERM; the same account outside it may: the control above).
        assert seen["abstract outside"] == "errno 1" and seen["abstract own"] == "ok", seen
        assert seen["signal outside"] == "errno 1", seen
        # A pair of sockets of unix only: TIPC's refused as a family it may not use (EAFNOSUPPORT).
        assert seen["pair unix"] == "made" and seen["pair tipc"] == "errno 97", seen
        # Avahi: refused on the bus, its socket not there.
        assert seen["avahi"]["bus"] != "answered" and "Access denied" in seen["avahi"]["bus"], seen
        assert seen["avahi"]["socket"] in ("PermissionError", "FileNotFoundError", []), seen
        machine.succeed("systemctl stop torq-test-outsider torq-test-abstract")
        machine.succeed("chmod 700 /home/alice")
        # The same, of the service's own view of the machine (its mount namespace, as the agent).
        inside = f"nsenter -t {prop('MainPID')} -m -- setpriv --reuid torq-agent-alice --regid torq-agent-alice --clear-groups"
        machine.fail(f"{inside} ls /home/alice")
        machine.fail(f"{inside} ls /run/user")
        machine.succeed(f"{inside} ls {home}")

    with subtest("a tool of TorqOS's, in plain words; what is not the engine's protocol changes nothing"):
        out = as_user("alice", "torq ask alarms 2>/tmp/err")
        assert out.strip() == "No alarms.", out
        assert "agent: looked at the alarms" in machine.succeed("cat /tmp/err")
        assert as_user("alice", "torq ask noise").strip() == "After the noise."

    with subtest("what it proposes: its number, alice's agent's at the broker, waiting for her"):
        out = as_user("alice", "torq ask 'install htop' 2>/tmp/err")
        m = re.search(r"Proposed #(\d+)", out)
        assert m and f"Proposed #{m.group(1)}: it waits for you" in out and f"request system apply {m.group(1)}" in out and "I proposed htop." in out, out
        err = machine.succeed("cat /tmp/err")
        assert "agent: proposed a change" in err, err
        assert "coding-tool" not in err and "taint" not in err and "taint" not in out, (out, err)
        p = [e for e in entries() if e["seq"] == int(m.group(1))][0]
        assert p["kind"] == "proposal" and p["actor"] == "agent" and p["data"]["agent"] == "torq-agent-alice" and p["data"]["agent_for"] == "alice", p
        assert p["tainted"] is True and p["data"]["session"]["tainted_by"][0] == "agent-program", p
        # Checked first (ADR-056): this stub repo evaluates neither as it is nor changed, so recorded as not checked.
        assert p["data"]["check"]["result"] == "not checked" and p["data"]["check"]["why"] == "the check could not be made on this machine right now", p
        assert not [e for e in entries() if e["kind"] == "applied"], "nothing applied: it waits for alice"

    with subtest("what the sandbox rests on, at run time: tracing only one's own descendants, no user manager for the agent"):
        # agent-assertions checks the configuration; this, the running machine.
        assert machine.succeed("sysctl -n kernel.yama.ptrace_scope").strip() in ("1", "2", "3")
        sibling = machine.succeed("setpriv --reuid torq-agent-alice --regid torq-agent-alice --clear-groups -- sleep 600 >/dev/null 2>&1 & echo $!").strip()
        machine.wait_until_succeeds(f"test \"$(ps -o user= -p {sibling})\" = torq-agent-alice", timeout=10)
        # The control: a program of the agent's traces its own child, which Yama allows.
        machine.succeed("setpriv --reuid torq-agent-alice --regid torq-agent-alice --clear-groups -- strace -f -e trace=none -o /dev/null true")
        # Same uid, not its descendant: Yama refuses the attach (with scope 0 it would be taken).
        machine.fail(f"setpriv --reuid torq-agent-alice --regid torq-agent-alice --clear-groups -- strace -p {sibling} -e trace=none -o /dev/null")
        machine.succeed(f"kill {sibling}")
        uid = machine.succeed("id -u torq-agent-alice").strip()
        machine.fail("test -e /var/lib/systemd/linger/torq-agent-alice")
        machine.fail(f"systemctl is-active user@{uid}.service")

    with subtest("no process of the agent's can put itself in the service's control group, or take one out"):
        assert prop("ControlGroup") == "/system.slice/system-torq\\x2dagent.slice/torq-agent@alice.service", prop("ControlGroup")
        procs = f"/sys/fs/cgroup{prop('ControlGroup')}/cgroup.procs"
        assert machine.succeed(f"stat -c %U {shlex.quote(procs)}").strip() == "root"
        machine.fail(f"su -s /bin/sh torq-agent-alice -c {shlex.quote(f'echo $$ > {shlex.quote(procs)}')}")
        as_user("alice", "torq agent run -- sh -c " + shlex.quote(f"echo $$ > {shlex.quote(procs)}"), ok=False)

    with subtest("bob cannot connect to alice's agent; the service answers alice alone, whoever connects"):
        out = machine.fail(f"su - bob -c 'socat -t 5 - UNIX-CONNECT:{sock} </dev/null' 2>&1")
        assert "Permission denied" in out, out
        # Root may connect (the kernel lets it): the service itself refuses it, by SO_PEERCRED.
        cmd = "printf '%s\\n' '{\"type\":\"status\"}' | socat -t 5 - UNIX-CONNECT:" + sock
        said = [json.loads(l) for l in machine.execute(cmd)[1].splitlines()]
        assert said == [{"type": "error", "text": "this is alice's agent: it answers alice alone"}, {"type": "done", "exit": 1}], said
        # And so bob, through a socket alice opened to everyone (she owns it): refused all the same.
        machine.succeed(f"su - alice -c 'chmod 666 {sock}'")
        assert raw("bob", {"type": "ask", "text": "hello"}) == said
        machine.succeed(f"chmod 600 {sock}")
        # The engine itself, the agent's account, cannot reach its own person's socket either.
        machine.fail(f"su -s /bin/sh torq-agent-alice -c 'socat -t 5 - UNIX-CONNECT:{sock} </dev/null'")
        # bob's own agent is his, and runs as his agent account, in his agent's home. He is no administrator, so
        # he cannot start `claude` to sign in (KNOWN-GAPS): he is told he is not signed in, and to ask one; signed in for
        # him here as root would have to, it answers him.
        code, out = machine.execute("su - bob -c 'torq ask hello' 2>&1")
        assert code != 0 and "Claude Code is not signed in for your agent" in out and "if you are not one, ask one" in out, out
        machine.succeed("setpriv --reuid torq-agent-bob --regid torq-agent-bob --clear-groups env HOME=/var/lib/torq-agent-bob /etc/profiles/per-user/torq-agent-bob/bin/claude --stand-in-login")
        assert as_user("bob", "torq ask hello").strip() == "Hello alice."
        assert machine.succeed("ps -o user= -p $(systemctl show -P MainPID torq-agent@bob.service)").strip() == "torq-agent-bob"
        assert starts("/var/lib/torq-agent-bob")[-1]["cwd"] == "/var/lib/torq-agent-bob"
        machine.succeed("systemctl stop torq-agent@bob.service")

    with subtest("Ctrl-C stops the answer: all of it, a program in a session of its own and one out of sight; the service stays, and the conversation"):
        before = raw("alice", {"type": "status"})[0]["turns"]
        machine.succeed("su - alice -c \"tmux new-session -d -s a -x 200 -y 50 'torq ask slow; echo exit=\\$?; sleep 600'\"")
        machine.wait_until_succeeds("pgrep -u torq-agent-alice -f 'sleep 601' && pgrep -u torq-agent-alice -f 'undumpable-602' && pgrep -u torq-agent-alice -f 'sleep 603'", timeout=60)
        assert raw("alice", {"type": "status"}) == status(True, before, 2)
        hidden = machine.succeed("pgrep -n -u torq-agent-alice -f 'undumpable-602'").strip()
        assert machine.succeed(f"stat -c %u /proc/{hidden}/status").strip() == "0", "undumpable: its /proc files are root's"
        own = machine.succeed("pgrep -n -u torq-agent-alice -f 'sleep 601'").strip()
        epid = machine.succeed(engine).strip()
        assert machine.succeed(f"ps -o sid= -p {own}").strip() != machine.succeed(f"ps -o sid= -p {epid}").strip(), "a session of its own"
        machine.succeed("su - alice -c 'tmux send-keys -t a C-c'")
        machine.wait_until_succeeds("su - alice -c 'tmux capture-pane -p -t a' | grep -q 'exit=1'", timeout=30)
        assert "agent: stopping" in machine.succeed("su - alice -c 'tmux capture-pane -p -t a'")
        for p in ["sleep 601", "undumpable-602", "sleep 603"]:
            machine.fail(f"pgrep -u torq-agent-alice -f '{p}'")
        machine.fail(engine)
        assert prop("ActiveState") == "active"
        assert machine.succeed(f"cat '/sys/fs/cgroup{prop('ControlGroup')}/cgroup.procs'").split() == [prop("MainPID")], "nothing but the service left in its unit"
        assert raw("alice", {"type": "status"}) == status(False, before, 2)
        # The conversation survived the stop: the engine is started again to continue it.
        assert as_user("alice", "torq ask 'remember?'").strip() == "You said hello."
        last = starts()[-1]["args"]
        assert last[-2] == "--resume" and last[-1] == json.loads(machine.succeed(f"cat {mem}/conversation.json"))["session"], last

    with subtest("an engine that ends in the middle of an answer: said, and the next question continues the conversation"):
        code, out = machine.execute("su - alice -c 'torq ask die' 2>&1")
        assert code != 0 and "half" in out and "Claude Code ended before it finished its answer. Ask again: the conversation is kept." in out, out
        machine.wait_until_fails(engine, timeout=30)
        assert as_user("alice", "torq ask 'remember?'").strip() == "You said hello."

    with subtest("the service restarted: the conversation is the one it was"):
        turns = raw("alice", {"type": "status"})[0]["turns"]
        machine.succeed(f"systemctl restart {unit}")
        # Nothing of the engine outlives the service either.
        machine.fail(engine)
        assert raw("alice", {"type": "status"}) == status(False, turns, 2)
        assert as_user("alice", "torq ask 'remember?'").strip() == "You said hello."
        assert "--resume" in starts()[-1]["args"]

    with subtest("a frozen service cannot hold the terminal: Ctrl-C twice, or once and 5 s, gets alice out"):
        # Each key sent once torq ask reads keys (its terminal out of canonical mode), and the second once the first
        # is taken (it says it is stopping): waited for, not slept.
        def reading_keys(s):
            tty = machine.succeed(f"su - alice -c 'tmux display -p -t {s} \"#{{pane_tty}}\"'").strip()
            machine.wait_until_succeeds(f"stty -F {tty} -a | grep -q -- -icanon", timeout=30)
        def pane_says(s, text, timeout):
            machine.wait_until_succeeds(f"su - alice -c 'tmux capture-pane -p -t {s}' | grep -q {shlex.quote(text)}", timeout=timeout)
        machine.succeed(f"systemctl freeze {unit}")
        machine.succeed("su - alice -c \"tmux new-session -d -s f -x 200 -y 50 'torq ask hello; echo exit=\\$?; sleep 600'\"")
        reading_keys("f")
        machine.succeed("su - alice -c 'tmux send-keys -t f C-c'")
        pane_says("f", "agent: stopping", 10)
        machine.succeed("su - alice -c 'tmux send-keys -t f C-c'")
        pane_says("f", "exit=130", 10)
        machine.succeed("su - alice -c \"tmux new-session -d -s g -x 200 -y 50 'torq ask hello; echo exit=\\$?; sleep 600'\"")
        reading_keys("g")
        machine.succeed("su - alice -c 'tmux send-keys -t g C-c'")
        pane_says("g", "agent: stopping", 10)
        # Not at once: it waits GIVE_UP (5 s) for the stop. That much is time, by its nature: 2 s, well inside it on a
        # slow machine.
        machine.sleep(2)
        machine.fail("su - alice -c 'tmux capture-pane -p -t g' | grep -q 'exit='")
        pane_says("g", "exit=130", 15)
        machine.succeed(f"systemctl thaw {unit}")
        machine.wait_until_succeeds("test \"$(su - alice -c 'torq ask hello')\" = 'Hello alice.'", timeout=60)

    with subtest("torq agent run beside the service and its engine: a program at a terminal runs, as the same account"):
        as_user("alice", "torq ask hello")
        machine.succeed(engine)
        out = json.loads(as_user("alice", "torq agent run -- claude --version").strip().splitlines()[-1])
        assert out == {"args": ["--version"], "user": "torq-agent-alice", "cwd": home}, out
        # The engine was left as it was.
        machine.succeed(engine)
        assert prop("FreezerState") == "running"

    with subtest("sharing while the service and its engine run: both frozen through the walk, thawed after"):
        assert prop("ActiveState") == "active" and prop("FreezerState") == "running"
        # The engine waits for the next question, as the agent, in the service's control group: held still
        # with it, and not taken for a session at a terminal (which would refuse the share).
        epid = machine.succeed(engine).strip()
        assert machine.succeed(f"cat /proc/{epid}/cgroup").strip() == "0::" + prop("ControlGroup")
        as_user("alice", "mkdir -p ~/project/sub && echo hi > ~/project/a && echo there > ~/project/sub/b")
        machine.succeed("rm -f /tmp/freezer-seen")
        as_user("alice", "torq agent run --share ~/project -- true")
        seen = machine.succeed("cat /tmp/freezer-seen").split()
        assert seen and all(s == "frozen" for s in seen), seen
        assert prop("FreezerState") == "running"
        assert "user:torq-agent-alice:rw" in machine.succeed("getfacl -p /home/alice/project/sub/b")
        # Shared with the agent account, and still out of the service's reach: its sandbox has no home in it.
        assert json.loads(as_user("alice", "torq ask look"))["/home"] in ("PermissionError", "FileNotFoundError", [])
        # A walk that fails: nothing shared, and the agent thawed all the same.
        machine.succeed("rm -f /tmp/freezer-seen && touch /tmp/fail-setfacl")
        as_user("alice", "torq agent run --share ~/project -- true", ok=False)
        assert machine.succeed("cat /tmp/freezer-seen").split()[0] == "frozen"
        assert prop("FreezerState") == "running"
        machine.succeed("rm /tmp/fail-setfacl")
        # The service answers after it, from the same engine.
        assert as_user("alice", "torq ask hello").strip() == "Hello alice."
        assert machine.succeed(engine).strip() == epid

    with subtest("claude typed plainly: Claude Code as her agent, in the folder she is in, every word passed on"):
        # TorqOS's own command, on her PATH: `torq agent claude`, which she could type herself. The tool itself
        # is in her agent's profile and not in hers.
        assert as_user("alice", "command -v claude").strip() == "/run/current-system/sw/bin/claude"
        machine.fail("test -e /etc/profiles/per-user/alice/bin/claude")
        machine.succeed("grep -q 'bin/torq agent claude \"$@\"' /run/current-system/sw/bin/claude")
        as_user("alice", "mkdir -p ~/work")
        words = ["--dangerously-skip-permissions", "--model", "a b", "it's \"quoted\" $HOME *", "--continue", "--", "-x"]
        out = json.loads(as_user("alice", "cd ~/work && " + shlex.join(["claude"] + words) + " 2>/tmp/err").strip().splitlines()[-1])
        # As her agent, in that folder, with every word as she typed it.
        assert out == {"args": words, "user": "torq-agent-alice", "cwd": "/home/alice/work"}, out
        # The first time the folder is shared, it is said, with how to take it back; after that, nothing.
        err = machine.succeed("cat /tmp/err")
        assert "Shared with your agent (torq-agent-alice): /home/alice/work" in err and "To take that back: setfacl -R -P -x u:torq-agent-alice,d:u:torq-agent-alice -- /home/alice/work" in err, err
        out = json.loads(as_user("alice", "cd ~/work && claude --again 2>/tmp/err").strip().splitlines()[-1])
        assert out["cwd"] == "/home/alice/work" and machine.succeed("cat /tmp/err") == "", machine.succeed("cat /tmp/err")
        # Typed out in full, it is the same command.
        out = json.loads(as_user("alice", "cd ~/work && torq agent claude --typed-out 2>/dev/null").strip().splitlines()[-1])
        assert out == {"args": ["--typed-out"], "user": "torq-agent-alice", "cwd": "/home/alice/work"}, out
        # From a script and another shell, its input passed on, and its exit status hers to read.
        out = json.loads(as_user("alice", "cd ~/work && printf 'from a pipe' | sh -c 'claude --stand-in-stdin' 2>/dev/null").strip().splitlines()[-1])
        assert out["stdin"] == "from a pipe" and out["cwd"] == "/home/alice/work", out
        code, _ = machine.execute("su - alice -c 'cd ~/work && claude --stand-in-exit 7' 2>/dev/null")
        assert code == 7, code

    with subtest("claude where the folder is never shared: it runs all the same, with nothing shared, and says so in one line"):
        as_user("alice", "mkdir -p ~/.ssh ~/.config/x && echo key > ~/.ssh/id")
        nothing = "Nothing is shared with your agent here ({}): it works in its own folder. To let it work on a project, go into that project's folder first (cd ~/<folder>)\n"
        for where, why in [
            ("~", "this is your home folder itself"),
            ("/etc", "this folder is outside your home"),
            ("~/.ssh", "this is a hidden folder of your home, where keys and settings live"),
            ("~/.config/x", "this is a hidden folder of your home, where keys and settings live"),
        ]:
            out = json.loads(as_user("alice", f"cd {where} && claude --version 2>/tmp/err").strip().splitlines()[-1])
            # It ran, as her agent, in the agent's own folder: nothing of hers bound in.
            assert out == {"args": ["--version"], "user": "torq-agent-alice", "cwd": home}, (where, out)
            err = machine.succeed("cat /tmp/err")
            assert err == nothing.format(why), (where, err)
            assert "--share" not in err, err
        # Nothing of those folders was given to the agent.
        for kept in ["/home/alice", "/home/alice/.ssh", "/home/alice/.ssh/id", "/home/alice/.config/x"]:
            assert "torq-agent-alice" not in machine.succeed(f"getfacl -p {kept}"), kept
        # Words passed on from anywhere, as the real one's `claude -p "hi"` would be (the stand-in only says how it was started).
        out = json.loads(as_user("alice", "cd ~ && claude --stand-in-plain -x hi 2>/dev/null").strip().splitlines()[-1])
        assert out["args"] == ["--stand-in-plain", "-x", "hi"] and out["cwd"] == home, out
        # Naming a hidden folder herself is refused, with why; nothing runs, nothing is shared.
        before = len(starts())
        out = machine.fail("su - alice -c 'torq agent run --share ~/.ssh -- true' 2>&1")
        assert "/home/alice/.ssh is a hidden folder of your home" in out and "Your keys, sign-ins and settings live there" in out and "share a folder of your own work instead" in out, out
        out = machine.fail("su - alice -c 'torq agent run --share ~ -- true' 2>&1")
        assert "is your home folder itself" in out and "or name one with --share" not in out, out
        assert "torq-agent-alice" not in machine.succeed("getfacl -p /home/alice/.ssh")
        assert len(starts()) == before

    with subtest("who cannot: someone with no agent, an agent's own account, someone who is not an administrator, each told why"):
        before = len(starts())
        out = machine.fail("cd /root && claude 2>&1")
        assert "you have no agent account on this machine" in out, out
        out = machine.fail("su -s /bin/sh torq-agent-alice -c 'cd /var/lib/torq-agent-alice && /run/current-system/sw/bin/claude' 2>&1")
        assert "an agent's account already" in out, out
        # bob has an agent and is no administrator: starting a program as it is not his today. He is told so in
        # one sentence, before sudo would ask him for a password it then refuses; who may is sudo's rule, as it was.
        machine.succeed("su - bob -c 'mkdir -p ~/work'")
        for cmd in ["cd ~/work && claude --version", "claude --version", "torq agent run -- true", "torq agent run --share ~/work -- true"]:
            out = machine.fail(f"su - bob -c {shlex.quote(cmd)} 2>&1 </dev/null")
            assert "Only an administrator of this machine can start a program as their agent today, and you are not one: ask an administrator." in out, (cmd, out)
            assert "password" not in out.lower() and "Nothing is shared" not in out, (cmd, out)
        assert "torq-agent-bob" not in machine.succeed("getfacl -p /home/bob/work")
        # The control, that it is sudo's rule and not only torqcli's word: the launcher itself, through sudo, is refused him.
        launcher = machine.succeed("readlink -f /run/current-system/sw/bin/torq-agent-run").strip()
        machine.fail(f"su - bob -c 'sudo -n {launcher} -- true' </dev/null")
        machine.succeed(f"su - alice -c 'sudo -n {launcher} -- true' </dev/null")
        assert len(starts()) == before, "none of them ran the tool"

    with subtest("a Claude Code of a person's own is found before TorqOS's command, and runs as them"):
        # carol installed one in her own profile (a stand-in): her PATH has her profile before the system's.
        assert as_user("carol", "command -v claude").strip() == "/etc/profiles/per-user/carol/bin/claude"
        assert as_user("carol", "claude").strip() == "carol's own, as carol"
        # The control: alice has none of her own, and gets TorqOS's (above).
        assert as_user("alice", "command -v claude").strip() == "/run/current-system/sw/bin/claude"
        # Inside the sandbox, `claude` is the tool itself (the agent's own profile first), not TorqOS's command again.
        assert as_user("alice", "torq agent run -- sh -c 'command -v claude'").strip() == "/etc/profiles/per-user/torq-agent-alice/bin/claude"

    with subtest("a second claude in another terminal: told to close the first, in plain words"):
        machine.succeed("su - alice -c \"tmux new-session -d -s c 'cd ~/work && claude --stand-in-sleep 600'\"")
        machine.wait_until_succeeds("pgrep -u torq-agent-alice -f 'bin/claude --stand-in-sleep 600'", timeout=30)
        out = machine.fail("su - alice -c 'cd ~/work && claude --version' 2>&1")
        assert "Claude Code is already open as your agent in another terminal: close that one first." in out, out
        assert "nothing was shared and nothing was started (the line above says why)" in out, out
        assert prop("FreezerState") == "running"
        machine.succeed("su - alice -c 'tmux kill-session -t c'")
        machine.wait_until_fails("pgrep -u torq-agent-alice -f 'stand-in-sleep'", timeout=30)
        # And the service answers after all of it.
        assert as_user("alice", "torq ask hello").strip() == "Hello alice."

    with subtest("instructions for your agent: her own file, given to the engine; a changed text restarts it in the same conversation"):
        instr = "/home/alice/.config/torq/agent-instructions.md"
        want = "[Standing instructions from the person this agent works for]\nAlways answer in German."
        agent_uid = int(machine.succeed("id -u torq-agent-alice"))
        # Given as a file the agent alone reads, never as words of a command line (every account reads those, and
        # sudo writes them to the journal): in the service, a file of the agent's own in its state; under `claude`,
        # one the launcher writes inside the sandbox.
        in_service = ["--append-system-prompt-file", f"{mem}/instructions.md"]
        in_sandbox = ["--append-system-prompt-file", "/run/torq-agent-instructions/standing.md"]
        told = {"text": want, "mode": "0o600", "uid": agent_uid}
        def no_text_anywhere():
            # Not on the command line of any process of the machine, nor in the journal (sudo's lines included).
            lines = machine.succeed("for p in /proc/[0-9]*; do tr '\\0' ' ' < $p/cmdline 2>/dev/null; echo; done")
            assert "answer in German" not in lines, [l for l in lines.splitlines() if "German" in l]
            assert "answer in German" not in machine.succeed("journalctl --no-pager -o cat"), "in the journal"
        out = as_user("alice", "torq agent instructions")
        assert f"Instructions for your agent: {instr}" in out and "None yet. `torq agent instructions edit`" in out, out
        # With none written, the engine is given none.
        as_user("alice", "torq ask hello")
        before = raw("alice", {"type": "status"})[0]
        epid = machine.succeed(engine).strip()
        assert not [a for a in starts()[-1]["args"] if a.startswith("--append-system-prompt")], starts()[-1]
        session = json.loads(machine.succeed(f"cat {mem}/conversation.json"))["session"]
        # She writes some. The file is hers, in a hidden folder of her home: her agent cannot write it, from its
        # account or from inside its sandbox.
        as_user("alice", f"mkdir -p ~/.config/torq && printf 'Always answer in German.\\n' > {instr}")
        assert "They are:\n  Always answer in German.\n" in as_user("alice", "torq agent instructions")
        machine.fail(f"setpriv --reuid torq-agent-alice --regid torq-agent-alice --clear-groups sh -c 'echo planted >> {instr}'")
        as_user("alice", f"torq agent run -- sh -c 'echo planted >> {instr}'", ok=False)
        assert machine.succeed(f"cat {instr}") == "Always answer in German.\n"
        # The next question brings them: the engine is ended and started again with them, continuing the same
        # conversation, and told to set aside its record of what the conversation began with (nothing).
        assert as_user("alice", "torq ask 'remember?'").strip() == "You said hello."
        last = starts()[-1]["args"]
        assert last[-6:] == ["--resume", session] + in_service + ["--system-prompt-snapshot", "off"], last
        assert starts()[-1]["told"] == told, starts()[-1]
        german = machine.succeed(engine).strip()
        assert german != epid, "another process"
        # The engine runs with them: their text is on no command line and in no journal.
        no_text_anywhere()
        machine.fail(f"su - bob -c 'cat {mem}/instructions.md'")
        assert raw("alice", {"type": "status"}) == status(False, before["turns"] + 1, before["conversation"])
        # The same instructions again: the same process.
        as_user("alice", "torq ask hello")
        assert machine.succeed(engine).strip() == german
        # Instructions that cannot be used (here too long): the question is asked without them, and that is said.
        as_user("alice", f"head -c 20000 /dev/zero | tr '\\0' x > {instr}")
        code, out = machine.execute("su - alice -c 'torq ask hello' 2>&1")
        assert code == 0 and "Hello alice." in out and "agent: Your instructions for your agent are not used: they are longer than 16 KiB: make them shorter" in out, out
        last = starts()[-1]["args"]
        # (The conversation began with none, so its own record is right again: continued, and nothing to set aside.)
        assert not [a for a in last if a.startswith("--append-system-prompt")] and last[-2:] == ["--resume", session], last
        assert "are not used: they are longer than 16 KiB" in as_user("alice", "torq agent instructions")
        # `claude`, typed plainly, gives them to Claude Code before her own words; `torq agent run -- claude` does not.
        as_user("alice", f"printf 'Always answer in German.\\n' > {instr}")
        out = json.loads(as_user("alice", "cd ~/work && claude --model x 2>/dev/null").strip().splitlines()[-1])
        assert out["args"] == in_sandbox + ["--model", "x"] and out["cwd"] == "/home/alice/work" and out["told"] == told, out
        out = json.loads(as_user("alice", "cd ~ && claude --version 2>/dev/null").strip().splitlines()[-1])
        assert out["args"] == in_sandbox + ["--version"] and out["told"] == told, out
        out = json.loads(as_user("alice", "torq agent run -- claude --version").strip().splitlines()[-1])
        assert out == {"args": ["--version"], "user": "torq-agent-alice", "cwd": home}, out
        # Inside the sandbox only, and gone after: nothing of it is left on the machine.
        machine.fail("test -e /run/torq-agent-instructions/standing.md")
        # Seen while `claude` runs: on no command line (sudo's, the launcher's, Claude Code's), in no journal.
        machine.succeed("su - alice -c \"tmux new-session -d -s t 'cd ~/work && claude --stand-in-sleep 30'\"")
        machine.wait_until_succeeds("pgrep -u torq-agent-alice -f 'stand-in-sleep'", timeout=30)
        no_text_anywhere()
        machine.succeed("su - alice -c 'tmux kill-session -t t'")
        machine.wait_until_fails("pgrep -u torq-agent-alice -f 'stand-in-sleep'", timeout=30)
        # Put back, the service's engine is given them again; removed, the conversation is continued as it began.
        as_user("alice", "torq ask hello")
        assert starts()[-1]["args"][-6:] == ["--resume", session] + in_service + ["--system-prompt-snapshot", "off"], starts()[-1]
        assert starts()[-1]["told"] == told, starts()[-1]
        assert "are removed" in as_user("alice", "torq agent instructions clear")
        machine.fail(f"test -e {instr}")
        assert "None yet" in as_user("alice", "torq agent instructions")
        as_user("alice", "torq ask hello")
        last = starts()[-1]["args"]
        assert last[-2:] == ["--resume", session] and not [a for a in last if a.startswith("--append-system-prompt")], last
        machine.fail(f"test -e {mem}/instructions.md")
        # An agent's own account has none to write.
        out = machine.fail("su -s /bin/sh torq-agent-alice -c 'cd /var/lib/torq-agent-alice && /run/current-system/sw/bin/torq agent instructions' 2>&1")
        assert "this is an agent's account" in out, out

    with subtest("torqcli killed in the middle of the walk: the agent is thawed all the same"):
        machine.succeed("touch /tmp/hang-setfacl")
        machine.succeed("su - alice -c \"tmux new-session -d -s k 'torq agent run --share ~/project -- true'\"")
        machine.wait_until_succeeds(f"test \"$(systemctl show -P FreezerState {unit})\" = frozen", timeout=30)
        machine.succeed("pkill -KILL -u alice -x torq")
        machine.wait_until_succeeds(f"test \"$(systemctl show -P FreezerState {unit})\" = running", timeout=30)
        machine.succeed("rm /tmp/hang-setfacl")
        machine.execute("su - alice -c 'tmux kill-session -t k'; pkill -u alice -f 'sleep 600'")

    with subtest("an agent program out of sight (undumpable) still makes a share refuse"):
        machine.succeed("su - alice -c \"tmux new-session -d -s u 'torq agent run -- ${undumpable} 604'\"")
        machine.wait_until_succeeds("pgrep -u torq-agent-alice -f 'undumpable 604'", timeout=30)
        hidden = machine.succeed("pgrep -n -u torq-agent-alice -f 'undumpable 604'").strip()
        assert machine.succeed(f"stat -c %u /proc/{hidden}/status").strip() == "0", "its /proc files are root's"
        out = machine.fail("su - alice -c 'torq agent run --share ~/project -- true' 2>&1")
        assert "A program is already running as your agent, in another terminal or left from earlier: close or end that one first." in out, out
        assert prop("FreezerState") == "running"
        machine.succeed("su - alice -c 'tmux kill-session -t u'")
        machine.wait_until_fails("pgrep -u torq-agent-alice -f 'undumpable 604'", timeout=30)

    with subtest("a torq agent run session running already: the share refuses, and nothing is left frozen"):
        machine.succeed("su - alice -c \"tmux new-session -d -s s 'torq agent run -- sleep 600'\"")
        machine.wait_until_succeeds("pgrep -u torq-agent-alice -x sleep", timeout=30)
        machine.succeed("rm -f /tmp/freezer-seen")
        out = machine.fail("su - alice -c 'torq agent run --share ~/project -- true' 2>&1")
        assert "A program is already running as your agent" in out and "close or end that one first" in out, out
        machine.fail("test -s /tmp/freezer-seen")
        assert prop("FreezerState") == "running"
        # And the service answers while that session runs: the two together.
        assert as_user("alice", "torq ask hello").strip() == "Hello alice."
        machine.succeed("su - alice -c 'tmux kill-session -t s'")
        machine.succeed("torq-broker verify")

    with subtest("the fence by its table, not its unit: a table taken away with the unit still up, and the agent's service does not start"):
        machine.succeed(f"systemctl stop {unit}")
        machine.succeed("nft delete table inet torq_agents")
        machine.succeed("systemctl is-active torq-agent-network.service")
        as_user("alice", "torq ask hello", ok=False)
        machine.fail(engine)
        assert "not in place" in machine.succeed(f"journalctl -u {unit} --no-pager"), "the reason, said in its journal"
        machine.succeed("systemctl reload torq-agent-network.service")
        assert as_user("alice", "torq ask hello").strip() == "Hello alice."
  '';
}
