# NixOS VM test: an agent's proposal is checked by the broker's checker (ADR-056), with a real evaluation.
#
# An agent's account is refused at the Nix daemon (modules/base.nix `allowed-users`; the audit of 2026-11-06,
# row 47), so torqcli as the agent cannot evaluate what it proposes: on a real machine every proposal of an
# agent's died there (seen on the owner's laptop, 2026-11-16), and no test saw it, because no VM evaluated a
# real configuration (KNOWN-GAPS 6zn). This one does: the machine's config repo is TorqOS's own template, its
# flake locked to this TorqOS and its nixpkgs in the VM's store, as first-boot setup writes it
# (modules/setup.nix), so `nix eval` of it runs with no network. Each claim fails if its guard is taken out:
# - (a) the agent's account is refused at the Nix daemon, plainly and under `torq agent run`; the owner is not;
# - (b) the real torqcli, as the agent, proposes a real package: the broker's checker evaluates the change, in
#   a unit of its own as `torq-check` with no network, and the ledger says it was checked, and on which tree,
#   the one applying it commits; a package that does not exist is refused in plain words and ledgered in one line;
# - (c) on a machine whose checker cannot reach the daemon (`torq-check` taken out of `allowed-users`), the
#   same proposal is recorded all the same, as not checked, and both the agent and the person are told.
{ pkgs, self }:

let
  # "Torq-sock-2210", as in the socket test.
  hash = "$6$torqsocketsalt$Z7RLey3l5vKGfukhfuZYYM/EjWNo7QM8.65r88XmZ9z3/jBCY.NAnH7RwYpiLoE2fedxUd/I7UFcKDX1KEXDg/";
  lib = pkgs.lib;
  src = self.sourceInfo;
  # The lock first-boot setup writes (modules/setup.nix `lock`): this TorqOS by its path in the store and its
  # hash, and every input it takes as it locks them, so nothing is fetched.
  torqLock = lib.importJSON "${src.outPath}/flake.lock";
  torqInputs = torqLock.nodes.${torqLock.root}.inputs;
  lock = builtins.toJSON {
    version = 7;
    root = "root";
    nodes = {
      root.inputs = { torq = "torq"; nixpkgs = [ "torq" "nixpkgs" ]; };
      torq = {
        inputs = torqInputs;
        locked = { type = "path"; path = "${src.outPath}"; narHash = src.narHash; lastModified = src.lastModified or 0; };
        original = { type = "path"; path = "${src.outPath}"; };
      };
    } // lib.genAttrs (lib.attrValues torqInputs) (n: torqLock.nodes.${n});
  };
  # The config repo as setup leaves it for the owner alice: the template, the platform, the lock, the image's disk.
  config = pkgs.runCommand "torq-agent-check-config" { inherit lock; passAsFile = [ "lock" ]; } ''
    cp -r ${self}/templates/config $out
    chmod -R u+w $out
    cp "$lockPath" $out/flake.lock
    cp ${../hosts/image/hardware-configuration.nix} $out/hardware-configuration.nix
    printf '{"user": "alice"}\n' > $out/nixos/torq.json
    sed -i -e 's|system = "aarch64-linux";|system = "${pkgs.stdenv.hostPlatform.system}";|' \
      -e 's|torq.url = "[^"]*";|torq.url = "path:${src.outPath}";|' $out/flake.nix
    grep -q 'path:/nix/store/' $out/flake.nix
  '';
  node = extra: { pkgs, lib, ... }: {
    imports = [ self.nixosModules.default extra ];
    torq.user = "alice";
    torq.desktop.enable = false;
    users.users.root.hashedPasswordFile = lib.mkForce null;
    users.users.alice.hashedPasswordFile = lib.mkForce null;
    users.users.alice.hashedPassword = hash;
    # A person who is no administrator.
    users.users.bob = { isNormalUser = true; hashedPasswordFile = lib.mkForce null; hashedPassword = hash; };
    torq.agent.people = [ "alice" ];
    torq.broker.rebuildCommand = "true";
    environment.systemPackages = [ pkgs.tmux ];
    # What an evaluation reads, in the VM's store and known to it: this TorqOS and every input it locks.
    system.extraDependencies = [ src.outPath ] ++ map (n: self.inputs.${n}.outPath) (lib.attrNames self.inputs);
    # An evaluation takes a gigabyte, and writes the system's derivations to the store: on a disk, not in memory.
    virtualisation.memorySize = 6144;
    virtualisation.cores = 4;
    virtualisation.diskSize = 8192;
    virtualisation.writableStoreUseTmpfs = false;
    nix.settings.min-free = lib.mkForce 0;
    # The test's own: without it each derivation written is synced to the VM's disk, and an evaluation that
    # takes a quarter of a minute on a real machine took four here (30 s of CPU), near the check's limit.
    nix.settings.fsync-store-paths = lib.mkForce false;
  };
in
pkgs.testers.runNixOSTest {
  name = "torq-agent-check";

  nodes.machine = node { };
  # A machine where the check cannot work: its checker's account is kept from the Nix daemon, as an agent's is.
  nodes.unchecked = node ({ lib, ... }: { nix.settings.allowed-users = lib.mkForce [ "root" "@wheel" ]; });

  testScript = ''
    import json, shlex
    repo = "/var/lib/torq/config"

    def set_up(m):
        m.wait_for_unit("multi-user.target")
        m.wait_for_unit("torq-broker.socket")
        m.succeed(f"mkdir -p {repo} && cp -r ${config}/. {repo}/ && chmod -R u+w {repo} && torq-broker init")
        m.fail("getent hosts cache.nixos.org")

    def entries(m):
        return [json.loads(l) for l in m.succeed("torq-broker log --json --brief").splitlines()]

    def run_as(m, who, cmd):
        return m.execute(f"su - {who} -s /bin/sh -c {shlex.quote(cmd)} 2>&1 < /dev/null")

    # The real torqcli, as alice's agent under `torq agent run`: a commit, which proposes and stops.
    def propose(package):
        script = f'configure; set users alice packages {package}; commit comment "{package} for alice"'
        return "torq agent run -- torqcli -c " + shlex.quote(script)

    start_all()
    set_up(machine)

    with subtest("(a) an agent's account is refused at the Nix daemon, plainly and under torq agent run; the owner is not"):
        allowed = machine.succeed("nix config show allowed-users").split()
        assert sorted(allowed) == ["@wheel", "root", "torq-check"], allowed
        trusted = machine.succeed("nix config show trusted-users").split()
        assert set(trusted) == {"root"}, trusted
        code, out = run_as(machine, "torq-agent-alice", "nix store info")
        assert code != 0 and "cannot open connection to remote store" in out, (code, out)
        code, out = run_as(machine, "alice", "torq agent run -- nix store info")
        assert code != 0 and "cannot open connection to remote store" in out, (code, out)
        code, out = run_as(machine, "alice", "torq agent run -- nix eval --raw --expr '\"x\"' && torq agent run -- nix build --no-link --expr 'derivation { name = \"x\"; system = \"x\"; builder = \"x\"; }'")
        assert code != 0, (code, out)
        code, out = run_as(machine, "alice", "nix store info")
        assert code == 0, (code, out)
        # A person who is no administrator: recorded here as it is (the open list: their own commit evaluates too).
        code, out = run_as(machine, "bob", "nix store info")
        print(f"a non-administrator at the Nix daemon: exit {code}: {out}")
        assert code != 0 and "cannot open connection to remote store" in out, (code, out)
        # The checker's account: no home, no shell, no password, its own group and no other.
        assert machine.succeed("id -Gn torq-check").split() == ["torq-check"]
        entry = machine.succeed("getent passwd torq-check").strip().split(":")
        assert entry[5] == "/var/empty" and entry[6].endswith("/nologin"), entry
        assert machine.succeed("getent shadow torq-check").split(":")[1] in ("!", "!!", "*"), "it has a password"

    with subtest("(b) the real torqcli as the agent proposes a real package: checked by torq-check, in a unit with no network"):
        n = len(entries(machine))
        machine.succeed(f"(su - alice -s /bin/sh -c {shlex.quote(propose('htop'))} > /tmp/propose.out 2>&1 < /dev/null; echo $? > /tmp/propose.code) >/dev/null 2>&1 &")
        # While it evaluates: the unit, as systemd has it, and its process, as the kernel has it.
        machine.wait_until_succeeds("test \"$(systemctl show torq-check.service -p ActiveState --value)\" = active", timeout=120)
        want = {"User": "torq-check", "PrivateNetwork": "yes", "NoNewPrivileges": "yes", "ProtectHome": "yes", "ProtectSystem": "strict", "PrivateTmp": "yes", "PrivateDevices": "yes", "CapabilityBoundingSet": "", "MemorySwapMax": "0", "TasksMax": "64", "RuntimeMaxUSec": "5min"}
        shown = machine.succeed("systemctl show torq-check.service " + " ".join(f"-p {p}" for p in want))
        got = dict(l.split("=", 1) for l in shown.splitlines())
        assert got == want, got
        pid = machine.succeed("systemctl show torq-check.service -p MainPID --value").strip()
        assert machine.succeed(f"stat -c %U /proc/{pid}").strip() == "torq-check"
        assert "--check-tree" in machine.succeed(f"tr '\\0' ' ' < /proc/{pid}/cmdline")
        assert machine.succeed(f"readlink /proc/{pid}/ns/net") != machine.succeed("readlink /proc/1/ns/net"), "it runs in the machine's network"
        links = machine.succeed(f"nsenter -t {pid} -n ip -o link").strip().splitlines()
        assert len(links) == 1 and " lo:" in links[0], links
        machine.wait_until_succeeds("test -e /tmp/propose.code", timeout=600)
        out = machine.succeed("cat /tmp/propose.out")
        assert machine.succeed("cat /tmp/propose.code").strip() == "0", out
        new = entries(machine)[n:]
        assert [e["kind"] for e in new] == ["proposal"], new
        p = new[0]
        assert f"proposed as #{p['seq']}: a person applies it (request system apply {p['seq']}). TorqOS checked it first: the configuration is valid with it." in out, out
        assert "cannot open connection" not in out and "error:" not in out, out
        check = p["data"]["check"]
        assert check["result"] == "valid" and isinstance(check["warnings"], list) and len(check["tree"]) == 40, check
        assert p["actor"] == "agent" and p["tainted"] is True and p["data"]["agent_for"] == "alice" and p["data"]["files"] == ["users/alice/packages.json"], p
        # The unit is gone when the check is: nothing of it stays.
        assert machine.succeed("systemctl show torq-check.service -p ActiveState --value").strip() == "inactive"
        # What was checked is, byte for byte, the tree applying it commits.
        machine.succeed(f"torq-broker apply {p['seq']} --approved-by alice --confirm-tainted")
        assert machine.succeed(f"git -C {repo} rev-parse 'HEAD^{{tree}}'").strip() == check["tree"]

    with subtest("(b) a package that does not exist is refused in plain words, and ledgered in one line"):
        n = len(entries(machine))
        # (A name torqcli itself takes, its set being one of nixpkgs': only the evaluation knows it is none.)
        code, out = run_as(machine, "alice", propose("python3Packages.nosuch-xyz"))
        assert code != 0, out
        assert 'refused: with this change the configuration is not valid: "python3Packages.nosuch-xyz" isn' in out and "t a package." in out, out
        assert "proposed as #" not in out and "cannot open connection" not in out and "show packages" not in out, out
        new = entries(machine)[n:]
        assert [e["kind"] for e in new] == ["refused"], new
        why = new[0]["data"]["why"]
        assert why == "refused: with this change the configuration is not valid: \"python3Packages.nosuch-xyz\" isn't a package.", why
        assert new[0]["data"]["agent"] == "torq-agent-alice", new[0]
        # A name that is in no way one, torqcli says itself, before anything is proposed.
        code, out = run_as(machine, "alice", propose("nosuch-package-xyz"))
        assert code != 0 and "is not a package in this machine's nixpkgs" in out and len(entries(machine)) == n + 1, (code, out)
        # The agent's other ways to Nix answer in plain words, never the daemon's: the search by name, the check.
        code, out = run_as(machine, "alice", "torq agent run -- torqcli show packages htop")
        assert code == 0 and "By name only" in out and "\n  htop\n" in out and "remote store" not in out, (code, out)
        code, out = run_as(machine, "alice", "torq agent run -- torqcli -c 'configure; set users alice packages jq; commit check'")
        assert code == 0 and "TorqOS checks an agent's change when it is proposed" in out and "remote store" not in out, (code, out)

    set_up(unchecked)

    with subtest("(c) where the checker cannot reach the daemon, the same proposal is recorded as not checked, and both are told"):
        assert sorted(unchecked.succeed("nix config show allowed-users").split()) == ["@wheel", "root"]
        code, out = run_as(unchecked, "alice", propose("htop"))
        assert code == 0, out
        new = [e for e in entries(unchecked) if e["kind"] in ("proposal", "refused")]
        assert [e["kind"] for e in new] == ["proposal"], new
        p = new[0]
        why = "this machine's configuration as it is does not pass the check either"
        assert p["data"]["check"]["result"] == "not checked" and p["data"]["check"]["why"] == why and len(p["data"]["check"]["tree"]) == 40, p
        assert f"proposed as #{p['seq']}: a person applies it (request system apply {p['seq']}). It could not be checked first ({why}): it may fail when applied." in out, out
        assert "cannot open connection" not in out and "error:" not in out, out
        # The person, before applying it.
        unchecked.succeed(f"su - alice -c \"tmux new-session -d -s a -x 250 -y 50 'torqcli request system apply {p['seq']}; sleep 600'\"")
        unchecked.wait_until_succeeds("su - alice -c 'tmux capture-pane -p -t a' | grep -q 'Apply?'", timeout=60)
        pane = unchecked.succeed("su - alice -c 'tmux capture-pane -p -J -t a'")
        assert f"TorqOS could not check this change beforehand ({why}): if it is not valid, applying it fails and changes nothing." in " ".join(pane.split()), pane
        unchecked.succeed("su - alice -c 'tmux send-keys -t a n Enter'")
  '';
}
