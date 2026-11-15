# NixOS VM test: TorqOS's desktop defaults reach a person who already has their own (the bug seen on the owner's
# laptop, 2026-11-15). COSMIC reads a person's own `custom` shortcuts and own `plugins_wings` instead of the system's, whole;
# alice has both, written before TorqOS's agent and its applet existed, as COSMIC writes them. After she logs in:
# Super+Shift+A opens the agent's window (a key in COSMIC's shortcut `defaults`, which cosmic-comp merges under her
# `custom`, left exactly as it was); the agent's applet is in her panel right after TorqOS's, her own applets
# kept and the running panel starting it; and when she takes it out, the next login (the unit run again) leaves it out.
# A file the program cannot read is left as it was.
{ pkgs, self }:

let
  # The owner's password, `torq-desktop`, as the broker keeps it (modules/accounts.nix).
  hash = "$6$torqtestsalt$wXsKSX3z/kTvGvAzmdrks4iI95ZmWWtQgfYUMV/OEbQ7h7HZxSpMJYh0hvWPma/izeabeQYfkEhhQhWA52xbE1";
  # Her files, in the style COSMIC writes (multi-line, trailing commas). No agent in either.
  custom = ''
    {
        (
            modifiers: [
                Super,
            ],
            key: "v",
            description: Some("Clip Keep"),
        ): Spawn("flatpak run io.github.marcelogomes90.cosmic-ext-applet-clip-keep --toggle"),
    }
  '';
  wings = ''
    Some(([
        "com.system76.CosmicPanelAppButton",
        "com.system76.CosmicAppletWorkspaces",
    ], [
        "org.torqos.Tunnels",
        "org.torqos.Applet",
        "com.system76.CosmicAppletStatusArea",
    ]))
  '';
  without = ''
    Some(([
        "com.system76.CosmicPanelAppButton",
    ], [
        "org.torqos.Applet",
        "com.system76.CosmicAppletStatusArea",
        "com.system76.CosmicAppletPower",
    ]))
  '';
in
pkgs.testers.runNixOSTest {
  name = "torq-desktop-defaults";
  # The desktop sets nixpkgs.overlays (COSMIC from its own nixpkgs), so the nodes build their own package set.
  node.pkgsReadOnly = false;

  nodes.machine = { lib, ... }: {
    imports = [ self.nixosModules.default ];
    torq.user = "alice";
    torq.desktop.enable = true;
    torq.desktop.dimInsteadOfSuspend = true;
    nixpkgs.hostPlatform = pkgs.stdenv.hostPlatform.system;
    users.users.root.hashedPasswordFile = lib.mkForce null;
    virtualisation.memorySize = 4096;
    torq.broker.rebuildCommand = "true";
    # Her agent exists, so that its window has an agent to talk of when the key opens it.
    torq.agent.people = [ "alice" ];
    environment.etc."torq-test/custom".text = custom;
    environment.etc."torq-test/plugins_wings".text = wings;
    environment.etc."torq-test/without".text = without;
  };

  testScript = ''
    import shlex
    machine.wait_for_unit("multi-user.target")
    machine.succeed("install -d -m 700 /var/lib/torq/secrets/passwords"
                    " && echo '${hash}' > /var/lib/torq/secrets/passwords/alice"
                    " && /run/current-system/activate")
    repo = "/var/lib/torq/config"
    machine.succeed(f"mkdir -p {repo} && echo '{{}}' > {repo}/desktop.json && touch {repo}/flake.nix && torq-broker init")
    cosmic = "/home/alice/.config/cosmic"
    shortcuts = f"{cosmic}/com.system76.CosmicSettings.Shortcuts/v1/custom"
    panel = f"{cosmic}/com.system76.CosmicPanel.Panel/v1/plugins_wings"
    unit = "torq-desktop-defaults.service"

    def as_alice(cmd):
        return machine.succeed(f"runuser -u alice -- sh -c {shlex.quote(cmd)}")

    def user_unit(args):
        return machine.execute(f"systemctl --user --machine=alice@ {args}")

    def journal():
        return machine.execute("journalctl --no-pager _SYSTEMD_USER_UNIT=torq-desktop-defaults.service")[1]

    with subtest("alice already has her own shortcut and her own panel, from before TorqOS had an agent"):
        as_alice(f"mkdir -p {cosmic}/com.system76.CosmicSettings.Shortcuts/v1 {cosmic}/com.system76.CosmicPanel.Panel/v1"
                 f" && cp /etc/torq-test/custom {shortcuts} && cp /etc/torq-test/plugins_wings {panel}")
        assert "org.torqos.Agent" not in as_alice(f"cat {panel}")

    with subtest("the owner logs in to COSMIC"):
        machine.wait_for_unit("graphical.target", timeout=180)
        machine.wait_until_succeeds("pgrep -u cosmic-greeter -f cosmic-greeter", timeout=60)
        for attempt in range(4):
            machine.sleep(15)
            machine.send_chars("torq-desktop\n", delay=0.2)
            if machine.execute("timeout 60 sh -c 'until pgrep -u alice -fx cosmic-panel; do sleep 2; done'")[0] == 0:
                break
        machine.wait_until_succeeds("pgrep -u alice -fx cosmic-panel", timeout=30)

    with subtest("the agent's applet is in her own panel after login, beside TorqOS's, and her own applets are as they were"):
        machine.wait_until_succeeds(f"grep -q org.torqos.Agent {panel}", timeout=120)
        got = as_alice(f"cat {panel}")
        order = [l.strip().strip(",").strip('"') for l in got.splitlines() if '"' in l]
        assert order == ["com.system76.CosmicPanelAppButton", "com.system76.CosmicAppletWorkspaces",
                         "org.torqos.Tunnels", "org.torqos.Applet", "org.torqos.Agent",
                         "com.system76.CosmicAppletStatusArea"], got
        assert "added org.torqos.Agent to your panel" in journal(), journal()
        # Remembered, per person, in her own state.
        assert "org.torqos.Agent" in as_alice("cat ~/.local/state/torq/desktop-defaults")
        # The running panel takes the change and starts the applet: no new login.
        machine.wait_until_succeeds("pgrep -u alice -fx torq-agent-applet", timeout=90)
        machine.sleep(5)
        machine.screenshot("panel")

    with subtest("Super+Shift+A opens the agent's window, though alice has shortcuts of her own; hers are as they were"):
        machine.succeed(f"runuser -u alice -- cmp /etc/torq-test/custom {shortcuts}")
        machine.send_key("meta_l-shift-a")
        machine.wait_until_succeeds("pgrep -u alice -f 'bin/torq-agent-ui$'", timeout=60)
        machine.sleep(3)
        machine.screenshot("agent-key")
        machine.succeed("pkill -u alice -f 'bin/torq-agent-ui$'")

    with subtest("she takes the agent's applet out; the next login leaves it out"):
        as_alice(f"cp /etc/torq-test/without {panel}")
        machine.sleep(10)
        code, out = user_unit(f"restart {unit}")
        assert code == 0, out
        machine.sleep(5)
        assert "org.torqos.Agent" not in as_alice(f"cat {panel}")
        machine.succeed(f"runuser -u alice -- cmp /etc/torq-test/without {panel}")
        assert journal().count("added org.torqos.Agent") == 1, journal()

    with subtest("a file the program cannot read is left exactly as it was, and the unit still ends well"):
        as_alice(f"printf 'Some((oops' > {panel}")
        code, out = user_unit(f"restart {unit}")
        assert code == 0, out
        assert as_alice(f"cat {panel}") == "Some((oops"
        assert "left as it was" in journal(), journal()
  '';
}
