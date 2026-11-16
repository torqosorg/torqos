# NixOS VM test: the agent's proposals (ADR-042, amended; ADR-055, phase 2). The broker holds each session's
# record where the agent cannot write; every session of an agent account's is marked from its start as one
# whose reading the broker cannot see, and every proposal of an agent's waits for a person, whatever session
# it names and whatever its caller says; a person applies one told plainly where it comes from; what could cut
# the machine off is applied only as a commit confirmed. And nothing applies an agent's proposal without a
# person: session trust is gone, its commands are no commands, and the old loop's second account does not exist.
#
# (torqcli's commit as the agent, and the broker's check of what it proposes, run for real on a real
# configuration in tests/agent-check.nix, ADR-056. This VM's config repo is a stub that does not evaluate,
# as it is or changed: every proposal here is recorded as not checked.)
{ pkgs, self }:

let
  # "Torq-sock-2210", as in the socket test.
  hash = "$6$torqsocketsalt$Z7RLey3l5vKGfukhfuZYYM/EjWNo7QM8.65r88XmZ9z3/jBCY.NAnH7RwYpiLoE2fedxUd/I7UFcKDX1KEXDg/";
in
pkgs.testers.runNixOSTest {
  name = "torq-agent-proposals";

  nodes.machine = { pkgs, lib, ... }: {
    imports = [ self.nixosModules.default ];
    torq.user = "dev";
    torq.desktop.enable = false;
    users.users.root.hashedPasswordFile = lib.mkForce null;
    users.users.dev.hashedPasswordFile = lib.mkForce null;
    users.users.dev.hashedPassword = hash;
    users.users.sam = { isNormalUser = true; hashedPasswordFile = lib.mkForce null; hashedPassword = hash; };
    torq.agent.people = [ "dev" "sam" ];
    torq.agent.mayPropose.settings = [ "networking" ];
    # The agents may read the system journal (off by default).
    torq.agent.journal = true;
    torq.broker.rebuildCommand = "true";
    environment.systemPackages = [ pkgs.socat pkgs.jq pkgs.tmux ];
  };

  testScript = ''
    import json, shlex
    machine.wait_for_unit("multi-user.target")
    machine.wait_for_unit("torq-broker.socket")
    repo = "/var/lib/torq/config"
    state = "/var/lib/torq/state"
    machine.succeed(f"mkdir -p {repo}/nixos && echo '{{}}' > {repo}/desktop.json && touch {repo}/flake.nix")
    machine.succeed(f"echo '{{\"wireless\": {{\"enable\": false}}}}' > {repo}/nixos/networking.json")
    machine.succeed("torq-broker init")
    PASSWORD = "Torq-sock-2210"

    def as_account(who, cmd):
        return machine.succeed(f"su -s /bin/sh {who} -c {shlex.quote(cmd)}")

    def ask(user, request):
        cmd = f"printf '%s\\n' {shlex.quote(json.dumps(request))} | socat -t 30 - UNIX-CONNECT:/run/torq/broker.sock"
        code, out = None, ""
        for l in as_account(user, cmd).splitlines():
            f = json.loads(l)
            out += f.get("out", "")
            code = f.get("exit", code)
        return code, (json.loads(out) if out.strip() else None)

    def head():
        return machine.succeed(f"git -C {repo} rev-parse HEAD").strip()

    def packages(person, names):
        new = json.dumps(names)
        code, cur = machine.execute(f"git -C {repo} show HEAD:users/{person}/packages.json")
        path = f"users/{person}/packages.json"
        if code != 0:
            return f"--- /dev/null\n+++ b/{path}\n@@ -0,0 +1 @@\n+{new}\n"
        return f"--- a/{path}\n+++ b/{path}\n@@ -1 +1 @@\n-{cur.strip()}\n+{new}\n"

    def propose(diff, reason, session=None, who="torq-agent-dev"):
        args = ["--json", "propose", f"--base={head()}", f"--reason={reason}"]
        args += [f"--session={session}"] if session else []
        return ask(who, {"args": args, "diff": diff})

    def entries():
        return [json.loads(l) for l in machine.succeed("torq-broker log --json --brief").splitlines()]

    def kinds(k):
        return [e for e in entries() if e["kind"] == k]

    def start(who="torq-agent-dev"):
        sid = as_account(who, "torqcli agent session start").strip()
        assert len(sid) == 32, sid
        return sid

    with subtest("one account for each person's agent, and none of the old loop's; the journal where the setting gives it"):
        machine.fail("getent passwd torq-resident-dev")
        machine.fail("getent passwd torq-resident-sam")
        machine.succeed("test \"$(id -Gn torq-agent-dev | tr ' ' '\\n' | sort | tr '\\n' ' ')\" = 'systemd-journal torq-agent-dev '")
        assert json.loads(machine.succeed("cat /etc/torq/agent.json")) == {"journal": True}
        settings = json.loads(machine.succeed("cat /etc/torq/broker.json"))
        assert "residents" not in settings and "model" not in settings, sorted(settings)
        machine.fail("test -e /run/current-system/sw/bin/torq-doctor")
        # The commands that went with the old loop are no commands.
        for gone in ["torq doctor 'why?'", "torq agent propose 'install htop'", "torqcli request system agent trust 30", "torqcli request system agent trust revoke"]:
            code, out = machine.execute(f"su - dev -c {shlex.quote(gone)} 2>&1 < /dev/null")
            assert code != 0, (gone, out)

    with subtest("a session the agent starts is the broker's: its record root's, the ledger untouched, marked from its start as one the broker cannot see into"):
        n = len(entries())
        sid = start()
        assert len(entries()) == n, "a session costs the ledger nothing until a proposal cites it"
        machine.succeed(f"test \"$(stat -c %a:%U {state}/sessions/{sid}.json)\" = 600:root")
        machine.fail(f"su -s /bin/sh torq-agent-dev -c 'cat {state}/sessions/{sid}.json'")
        machine.fail(f"su -s /bin/sh torq-agent-dev -c 'touch {state}/sessions/x.json'")
        rec = json.loads(machine.succeed(f"cat {state}/sessions/{sid}.json"))
        # (The record says what the session read, `agent-program` first; it has no flag of its own beside that.)
        assert "tainted" not in rec and rec["tainted_by"] == ["agent-program"] and rec["agent"] == "torq-agent-dev" and "resident" not in rec, rec
        # A person's reads are their own: sessions are the agent's.
        code, v = ask("dev", {"args": ["--json", "session-start"]})
        assert code == 1 and "the agent's" in v["error"], (code, v)
        code, v = ask("dev", {"args": ["--json", "session-taint", sid, "--source=read_journal"]})
        assert code == 1, (code, v)

    with subtest("every proposal of an agent's is tainted, whatever session it names and whatever its caller says"):
        code, first = propose(packages("dev", ["htop"]), "htop for dev", session=sid)
        assert code == 0 and first["tainted"] is True and first["data"]["session"]["id"] == sid and first["data"]["session"]["tainted_by"] == ["agent-program"], (code, first)
        assert first["data"]["agent"] == "torq-agent-dev" and first["data"]["agent_for"] == "dev" and "resident" not in first["data"] and "trust_applied" not in first, first
        code, v = propose(packages("dev", ["htop"]), "htop, no session")
        assert code == 0 and v["tainted"] is True, ("no session: tainted", v)
        code, v = propose(packages("dev", ["htop"]), "htop, no such session", session="0" * 32)
        assert code == 1 and "no session the broker holds" in v["error"], (code, v)
        assert kinds("refused")[-1]["actor"] == "agent"
        # What it reports reading is added, for good; nothing the agent can send clears it; the ledger says so
        # where a proposal cites it.
        as_account("torq-agent-dev", f"torqcli agent session taint {sid} read_journal")
        as_account("torq-agent-dev", f"torqcli agent session taint {sid} show_ledger")
        code, jq = propose(packages("dev", ["jq"]), "jq for dev", session=sid)
        assert code == 0 and jq["tainted"] is True and jq["data"]["session"]["tainted_by"] == ["agent-program", "read_journal", "show_ledger"], jq
        for args in [["session-start", "--clear"], ["session-clear", sid], ["session-taint", sid, "--source=x;y"], ["session-start", "--resident"]]:
            code, v = ask("torq-agent-dev", {"args": ["--json"] + args})
            assert code != 0, (args, code, v)
        rec = json.loads(machine.succeed(f"cat {state}/sessions/{sid}.json"))
        assert "tainted" not in rec and rec["tainted_by"] == ["agent-program", "read_journal", "show_ledger"], rec
        # What the agent is told it proposed is the broker's record of the session.
        told = json.loads(as_account("torq-agent-dev", f"torqcli agent session show {sid}"))
        assert told["proposals"] == [{"proposal": first["seq"]}, {"proposal": jq["seq"]}], told
        # Nothing was applied: each waits.
        assert kinds("applied") == [], kinds("applied")
        out = machine.succeed("su - dev -c 'torqcli show system proposals'")
        assert f"#{jq['seq']}" in out and "torq-agent-dev" in out and "taint" not in out, out

    with subtest("one agent never cites another's session, nor proposes in another person's folder"):
        code, v = propose(packages("sam", ["bat"]), "sam's folder, by dev's agent", session=sid)
        assert code == 1 and "only for dev" in v["error"], (code, v)
        ss = start("torq-agent-sam")
        code, v = propose(packages("sam", ["bat"]), "sam's folder, sam's agent", session=ss, who="torq-agent-sam")
        assert code == 0 and v["tainted"] is True and v["data"]["agent"] == "torq-agent-sam", (code, v)
        code, v = propose(packages("sam", ["bat"]), "sam's agent, dev's agent's session", session=sid, who="torq-agent-sam")
        assert code == 1 and "not torq-agent-sam's" in v["error"], (code, v)
        code, out = machine.execute(f"su -s /bin/sh torq-agent-sam -c 'torqcli agent session taint {sid} web' 2>&1")
        assert code != 0 and "not torq-agent-sam's" in out, (code, out)

    with subtest("an apply needs the person's own word for it; a person applies it, told plainly where it comes from"):
        # Not applied by an apply that does not say the person looked (the broker's own rule, whoever asks).
        code, v = ask("dev", {"args": ["--json", "apply", str(jq["seq"])]})
        assert code == 1 and "--confirm-tainted" in v["error"], (code, v)
        n = jq["seq"]
        machine.succeed(f"su - dev -c \"tmux new-session -d -s a -x 200 -y 50 'torqcli request system apply {n}; sleep 600'\"")
        pane = lambda s: machine.succeed(f"su - dev -c 'tmux capture-pane -p -J -t {s}'")
        machine.wait_until_succeeds("su - dev -c 'tmux capture-pane -p -t a' | grep -q 'Apply?'", timeout=60)
        said = pane("a")
        assert "Your agent proposed this." in said and "anyone could have written (the system's log, the record of changes)" in said and "not the reason" in said, said
        assert "taint" not in said and "agent-program" not in said and "read_journal" not in said, said
        machine.succeed("su - dev -c 'tmux send-keys -t a y Enter'")
        machine.wait_until_succeeds("su - dev -c 'tmux capture-pane -p -t a' | grep -q 'applied proposal'", timeout=60)
        e = kinds("applied")[-1]
        assert e["data"]["proposal"] == jq["seq"] and e["tainted"] is True and e["approved_by"] == "dev" and "trust" not in e["data"], e
        machine.succeed(f"git -C {repo} show HEAD:users/dev/packages.json | grep -q jq")
        out = machine.succeed("su - dev -c 'torqcli show system ledger'")
        assert "proposed by an agent" in out and "tainted" not in out, out

    with subtest("what could cut the machine off is applied only as a commit confirmed"):
        s2 = start()
        wifi = "--- a/nixos/networking.json\n+++ b/nixos/networking.json\n@@ -1 +1 @@\n-{\"wireless\": {\"enable\": false}}\n+{\"wireless\": {\"enable\": true}}\n"
        code, w = propose(wifi, "wifi on", session=s2)
        assert code == 0 and w["data"]["cuts_off"] == ["networking.wireless.enable"], (code, w)
        code, v = ask("dev", {"args": ["--json", "apply", str(w["seq"]), "--confirm-tainted"], "password": PASSWORD})
        assert code == 1 and "commit confirmed" in v["error"], (code, v)
        # torqcli says so, and would apply it confirmed (answered no here: the VM cannot build a system).
        n = w["seq"]
        machine.succeed(f"su - dev -c \"tmux new-session -d -s w -x 200 -y 50 'torqcli request system apply {n}; sleep 600'\"")
        machine.wait_until_succeeds("su - dev -c 'tmux capture-pane -p -t w' | grep -q 'Apply?'", timeout=60)
        assert "could cut this machine off" in pane("w") and "commit confirmed" in pane("w"), pane("w")
        machine.succeed("su - dev -c 'tmux send-keys -t w n Enter'")
        # Nor at a set time: an agent's proposal is applied by a person who is there.
        at = machine.succeed("date -u -d '+1 hour' +%Y-%m-%dT%H:%M:%SZ").strip()
        code, v = ask("dev", {"args": ["--json", "schedule", str(first["seq"]), f"--at={at}"], "password": PASSWORD})
        assert code == 1 and "is an agent's" in v["error"], (code, v)

    with subtest("there is no trust to give: not a person's with their password, not the agent's, not root's"):
        for who, request in [
            ("dev", {"args": ["--json", "trust-grant", "--minutes=30"], "password": PASSWORD}),
            ("dev", {"args": ["--json", "trust-revoke"]}),
            ("dev", {"args": ["--json", "trust"]}),
            ("torq-agent-dev", {"args": ["--json", "trust-grant", "--minutes=30"]}),
        ]:
            code, v = ask(who, request)
            assert code not in (0, None), (who, request, code, v)
        machine.fail("torq-broker trust-grant --minutes 30 --approved-by dev")
        machine.fail("torq-broker trust")
        assert kinds("trust_granted") == []
        assert '"trust"' not in machine.succeed("torq-broker --json status || true"), "the status says nothing of trust"
        before = head()
        code, v = propose(packages("dev", ["jq", "tree"]), "tree for dev", session=s2)
        assert code == 0 and v["tainted"] is True and "trust_applied" not in v, (code, v)
        assert head() == before, "it waits"
        machine.succeed("torq-broker verify")
  '';
}
