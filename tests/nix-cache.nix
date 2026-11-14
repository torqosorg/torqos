# NixOS VM test: the broker repairs Nix's own caches (KNOWN-GAPS 6t). A power cut can leave a cache under the
# broker's HOME (/root/.cache/nix) damaged; every build after fails with "file is not a database". The broker removes
# the folder and runs the build once more. Here: that repair, and a power cut in the middle of it.
#
# A VM test cannot build systems, so the broker's "rebuild" is a script that does what in a build touches the cache:
# `nix flake metadata` of the config repo at the commit (a git fetch, which reads and writes the fetcher cache). It
# stops, when told to, at the point after the broker has removed the damaged folder and before it runs again; that
# stop is in the test's script, not in the broker.
{ pkgs, self }:

let
  rebuild = pkgs.writeShellScript "torq-test-rebuild" ''
    export PATH=${pkgs.git}/bin:$PATH
    echo run >> /var/lib/torq-test/runs
    # The second run of a repair: the damaged folder is gone. Say so, and wait to be cut off.
    if [ -e /var/lib/torq-test/hold ] && [ ! -e /root/.cache/nix ]; then
      touch /var/lib/torq-test/held
      sync
      sleep 3600
    fi
    exec nix --extra-experimental-features 'nix-command flakes' flake metadata --offline "$1"
  '';
in
pkgs.testers.runNixOSTest {
  name = "torq-nix-cache";

  nodes.machine = { lib, pkgs, ... }: {
    imports = [ self.nixosModules.default ];
    torq.user = "dev";
    torq.desktop.enable = false;
    users.users.root.hashedPasswordFile = lib.mkForce null;
    environment.systemPackages = [ pkgs.jq pkgs.git ];
    torq.broker.rebuildCommand = "${rebuild} {flake}";

    environment.etc."torq-test/desktop.nix".text = ''
      { nightLight = false; }
    '';
    environment.etc."torq-test/night-light.diff".text = ''
      --- a/desktop.nix
      +++ b/desktop.nix
      @@ -1 +1 @@
      -{ nightLight = false; }
      +{ nightLight = true; }
    '';
    environment.etc."torq-test/night-light-off.diff".text = ''
      --- a/desktop.nix
      +++ b/desktop.nix
      @@ -1 +1 @@
      -{ nightLight = true; }
      +{ nightLight = false; }
    '';
  };

  testScript = ''
    import json

    repo = "/var/lib/torq/config"
    cache = "/root/.cache/nix"
    machine.wait_for_unit("multi-user.target")

    def broker(args):
        return json.loads(machine.succeed(f"torq-broker --json {args}"))
    def runs():
        return int(machine.succeed("wc -l < /var/lib/torq-test/runs").strip())
    def damage():
        # The state a power cut leaves: a cache file that is not a database. Made by running the build's own command
        # once by hand, so that the files are the ones Nix really writes there.
        head = machine.succeed(f"git -C {repo} rev-parse HEAD").strip()
        machine.succeed(f"${rebuild} 'git+file://{repo}?rev={head}' && rm -f /var/lib/torq-test/runs")
        files = machine.succeed(f"ls {cache}/*.sqlite").split()
        assert files, "the build left no cache file: the test's rebuild command does not reach Nix's cache"
        for f in files:
            machine.succeed(f"head -c 4096 /dev/urandom > {f}")
        # The premise of the test: this command, on this cache, fails the way the broker looks for.
        status, out = machine.execute(f"${rebuild} 'git+file://{repo}?rev={head}' 2>&1")
        assert status != 0 and "(in '" + cache + "/" in out, (status, out)
        assert "file is not a database" in out or "malformed" in out, out
        machine.succeed("rm -f /var/lib/torq-test/runs")

    machine.succeed("mkdir -p /var/lib/torq-test")
    machine.succeed(f"mkdir -p {repo} && cp /etc/torq-test/desktop.nix {repo}/ && echo '{{ outputs = _: {{ }}; }}' > {repo}/flake.nix")
    machine.succeed("torq-broker init")

    with subtest("a build that finds Nix's cache damaged removes it and runs once more"):
        damage()
        p = broker("propose --diff /etc/torq-test/night-light.diff --reason 'Night light'")["seq"]
        a = broker(f"apply {p} --approved-by tester")
        assert a["kind"] == "applied", a
        assert runs() == 2, runs()
        machine.succeed("torq-broker verify")

    with subtest("a power cut after the damaged cache is removed, before the build runs again, is mended"):
        damage()
        p = broker("propose --diff /etc/torq-test/night-light-off.diff --reason 'Night light off'")["seq"]
        machine.succeed("touch /var/lib/torq-test/hold && sync")
        machine.succeed(f"(torq-broker apply {p} --approved-by tester > /tmp/apply.log 2>&1 &)")
        # The repair has removed the folder and is about to run the build again.
        machine.wait_until_succeeds("test -e /var/lib/torq-test/held", timeout=120)
        machine.fail(f"test -e {cache}")
        assert runs() == 2, runs()
        machine.crash()
        machine.start()
        machine.wait_for_unit("multi-user.target")
        # The broker's door is up (the unit's text, not a property of it: `systemctl show -P` can print [unprintable]).
        machine.wait_for_unit("torq-broker.socket")
        assert "ListenStream=/run/torq/broker.sock" in machine.succeed("systemctl cat torq-broker.socket")
        # The cut apply was not finished: the broker says so and refuses the next change until it is reconciled.
        status, out = machine.execute("torq-broker status 2>&1")
        assert "interrupted" in out, (status, out)
        machine.fail(f"torq-broker apply {p} --approved-by tester")
        # No stop this time.
        machine.succeed("rm -f /var/lib/torq-test/hold /var/lib/torq-test/held")
        r = broker("reconcile --approved-by tester")
        assert r["reconciled"]["kind"] == "interrupted", r
        # The same command that was cut now goes through, whatever state the cache came back in.
        a = broker(f"apply {p} --approved-by tester")
        assert a["kind"] == "applied", a
        machine.succeed(f"grep -q 'nightLight = false' {repo}/desktop.nix")
        machine.succeed("torq-broker verify")
  '';
}
