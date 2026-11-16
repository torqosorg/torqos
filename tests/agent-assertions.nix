# Evaluation check (builds nothing): modules/agent.nix's assertions each refuse what they say they
# refuse, with their message, and a plain configuration trips none of them. If an assertion is taken
# out or stops matching, evaluating this check throws.
{ pkgs, self }:

let
  lib = pkgs.lib;
  base = {
    torq.user = "dev";
    boot.loader.grub.enable = false;
    fileSystems."/" = { device = "/dev/disk/by-label/root"; fsType = "ext4"; };
  };
  configOf = extra: (import (pkgs.path + "/nixos/lib/eval-config.nix") {
    system = pkgs.stdenv.hostPlatform.system;
    modules = [ self.nixosModules.default base extra ];
  }).config;
  failing = extra:
    let
      config = (import (pkgs.path + "/nixos/lib/eval-config.nix") {
        system = pkgs.stdenv.hostPlatform.system;
        modules = [ self.nixosModules.default base extra ];
      }).config;
    in map (a: a.message) (lib.filter (a: !a.assertion) config.assertions);
  # An engine chosen (ADR-055): Claude Code allowed, installed for the agents (a stand-in: the real one is
  # unfree and large) and named as the engine — with whatever else `extra` says.
  withEngine = extra: {
    imports = [ extra ];
    torq.unfree.packages = [ "claude-code" ];
    torq.agent.tools = [ "claude-code" ];
    torq.agent.engine = "claude-code";
    torq.agent.claudeCode = pkgs.hello;
  };
  ours = ms: lib.filter (m: lib.hasInfix "torq.agent" m || lib.hasInfix "torq-agent" m || lib.hasInfix "torq-check" m || lib.hasInfix "ptrace_scope" m) ms;
  # `name` fails evaluation with a message holding `text`.
  refuses = name: extra: text:
    let ms = failing extra; in
    if lib.any (lib.hasInfix text) ms then name
    else throw "agent-assertions: ${name}: no assertion says \"${text}\" (failing: ${builtins.toJSON (ours ms)})";
  cases = [
    (let ms = ours (failing { }); in if ms == [ ] then "plain" else throw "agent-assertions: a plain configuration fails: ${builtins.toJSON ms}")
    (refuses "not a person" { torq.agent.people = [ "ghost" ]; } "ghost is not a person on this machine")
    (refuses "twice" { torq.agent.people = [ "dev" "dev" ]; } "a person is named twice")
    (refuses "a person in people named as an agent" { torq.agent.people = [ "torq-agent-x" ]; users.users.torq-agent-x.isNormalUser = true; } "torq-agent-x is named as an agent account is")
    (refuses "any person named as an agent" { users.users.torq-agent-y.isNormalUser = true; } "torq-agent-y is a person named as an agent account is")
    # The name's limit is measured against the account made for the person: `torq-agent-<name>` (20 characters
    # of name).
    (refuses "too long for an agent account" { torq.agent.people = [ "dev" "abcdefghijklmnopqrstu" ]; users.users.abcdefghijklmnopqrstu.isNormalUser = true; } "torq-agent-abcdefghijklmnopqrstu (the account made for abcdefghijklmnopqrstu's agent) is longer than an account's and its group's name may be (31 characters on NixOS): abcdefghijklmnopqrstu is too long a name for an agent of its own (at most 20 characters)")
    (let ms = ours (failing { torq.agent.people = [ "dev" "abcdefghijklmnopqrst" ]; users.users.abcdefghijklmnopqrst.isNormalUser = true; }); in
      if ms == [ ] then "20 characters fit an agent account" else throw "agent-assertions: a name of 20 characters is refused: ${builtins.toJSON ms}")
    # Removed with TorqOS's own agent loop (ADR-055, phase 2): a configuration that still sets one of its
    # settings is refused in words that say what to delete, never as an option NixOS does not know. Each
    # subtree is one removed option, so any setting under it is caught. What to type comes first (torqcli
    # shows a refusal's first 300 characters); only what is set is named; the journal's new place is said
    # only where the journal was on, with the truth about when it can be set.
    (refuses "doctor mode still set" { torq.agent.doctor.enable = true; } "`torq agent doctor` is still set, in ")
    (refuses "doctor mode still set, said" { torq.agent.doctor = { enable = true; endpoint = "https://api.example.org/v1"; model = "m"; }; } "and TorqOS no longer has it. In torqcli: `configure`, `delete torq agent doctor`, `commit`")
    (refuses "the journal under doctor mode, said" { torq.agent.doctor.journal = true; } "That setting is now `torq agent journal`")
    (refuses "the journal: when it can be set" { torq.agent.doctor.journal = true; } "own, after the update. Until then the agents do not read the journal.")
    (refuses "a key file still set" { torq.agent.doctor.apiKeyFile = "/var/lib/torq/secrets/model.key"; } "TorqOS's own agent loop is gone")
    (refuses "the local model still set" { torq.agent.model = { url = "https://example.org/m.gguf"; hash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="; }; } "`delete torq agent model`")
    (refuses "the local model's flags still set" { torq.agent.model.extraFlags = [ "--ctx-size" "16384" ]; } "`torq agent model` is still set, in ")
    (let
      said = ms: lib.filter (lib.hasInfix "is still set") ms;
      off = said (failing { torq.agent.doctor = { enable = true; journal = false; }; });
      doctorOnly = said (failing { torq.agent.doctor.enable = true; });
      # The same, set where a machine sets it: the tree's own file, which the words name.
      inTheTree = said (failing { imports = [ (import ../modules/nixos-settings.nix ./removed-settings) ]; });
    in
      if lib.any (lib.hasInfix "journal") (off ++ doctorOnly) then throw "agent-assertions: the journal is spoken of where it was not on: ${builtins.toJSON off}"
      else if lib.length doctorOnly != 1 || lib.any (lib.hasInfix "torq agent model") doctorOnly then throw "agent-assertions: a setting that is not set is named to delete: ${builtins.toJSON doctorOnly}"
      else if !(lib.any (lib.hasInfix "is still set, in `nixos/torq.json', and") inTheTree) then throw "agent-assertions: the file a removed setting is in is not named: ${builtins.toJSON inTheTree}"
      else if lib.any (m: lib.stringLength (lib.head (lib.splitString "\n" m)) > 200) inTheTree then throw "agent-assertions: what to type is not within what torqcli shows of a refusal"
      else "a removed setting: what is set, where, and what to type")
    # The journal, the agents' own setting now: off by default; on, each agent account is in systemd-journal
    # and in no other group, and torq-mcp is told so. No local model's service, no second account, either way.
    (let off = configOf { }; on = configOf { torq.agent.journal = true; }; in
      if off.users.users.torq-agent-dev.extraGroups == [ ] && on.users.users.torq-agent-dev.extraGroups == [ "systemd-journal" ]
        && builtins.fromJSON off.environment.etc."torq/agent.json".text == { journal = false; }
        && builtins.fromJSON on.environment.etc."torq/agent.json".text == { journal = true; }
        && !(on.services.llama-cpp.enable or false) && !(on.users.users ? torq-resident-dev)
      then "the journal is the agents' own setting"
      else throw "agent-assertions: torq.agent.journal does not give the agents the journal, and only that")
    (refuses "SETENV on the launcher's rule" ({ config, ... }: {
      security.sudo.extraRules = [{ groups = [ "wheel" ]; commands = [{ command = "${config.torq.agent.launcher}/bin/torq-agent-run"; options = [ "NOPASSWD" "SETENV" ]; }]; }];
    }) "a rule for torq-agent-run with SETENV")
    # torq agent tools (ADR-051 5): Claude Code is unfree, so refused until `torq unfree` allows it — and then nothing
    # unfree is put in the agent's packages; allowed, it is there, in the agent's own profile, not the system's.
    (refuses "claude-code not allowed" { torq.agent.tools = [ "claude-code" ]; } "claude-code is unfree")
    (let c = configOf { torq.agent.tools = [ "claude-code" ]; }; in
      if c.users.users.torq-agent-dev.packages == [ ] then "claude-code not allowed: not installed" else throw "agent-assertions: claude-code put in the agent's packages unallowed")
    (let
      extra = { torq.agent.tools = [ "claude-code" ]; torq.unfree.packages = [ "claude-code" ]; };
      c = configOf extra;
      names = map lib.getName c.users.users.torq-agent-dev.packages;
      ms = ours (failing extra);
    in
      if ms == [ ] && names == [ "claude-code" ] && !(lib.any (p: lib.getName p == "claude-code") c.environment.systemPackages)
      then "claude-code allowed: the agent's alone"
      else throw "agent-assertions: claude-code allowed: ${builtins.toJSON { inherit ms names; }}")
    # The engine (ADR-055 1): one of the coding tools installed for the agents, or refused, saying what to set.
    (refuses "an engine not installed" { torq.agent.engine = "claude-code"; } "torq.agent.engine: claude-code is not installed for the agents. Install it first: `set torq agent tools claude-code`")
    (let ms = ours (failing (withEngine { })); in if ms == [ ] then "an engine installed" else throw "agent-assertions: an engine that is installed is refused: ${builtins.toJSON ms}")
    # The agent's service is where an engine is chosen, and nowhere else: not on a plain machine. And no
    # second account for the agent anywhere (the old loop's `torq-resident-<person>`, removed).
    (let plain = configOf { }; engine = configOf (withEngine { }); has = c: c.systemd.services ? "torq-agent@dev" || c.systemd.sockets ? "torq-agent@dev"; in
      if !(has plain) && has engine
        && !(plain.users.users ? torq-resident-dev) && !(engine.users.users ? torq-resident-dev)
        && (let settings = builtins.fromJSON (builtins.unsafeDiscardStringContext engine.environment.etc."torq/broker.json".text); in !(settings ? residents) && !(settings ? model) && settings.agent.agents == { torq-agent-dev = "dev"; })
        && (builtins.fromJSON engine.environment.etc."torq/agent-tools.json".text) == { tools = [ "claude-code" ]; engine = "claude-code"; plainCommand = true; }
        && (builtins.fromJSON plain.environment.etc."torq/agent-tools.json".text) == { tools = [ ]; engine = null; plainCommand = false; }
      then "the service is where an engine is"
      else throw "agent-assertions: the agent's service is not exactly where an engine is chosen")
    # The old switch for the service is gone, and says what took its place.
    (refuses "the old service switch" { torq.agent.service.enable = false; } "torq.agent.engine")
    # The unit is the bridge, as the person's one agent account, in the launcher's sandbox and more: who it
    # runs as, what it runs, what it may write, what it cannot see. (The whole text is held by the rule below
    # and tests/agent-units.nix; this says the lines the fence is.)
    (let sc = (configOf (withEngine { })).systemd.services."torq-agent@dev".serviceConfig; in
      if sc.User == "torq-agent-dev" && sc.Group == "torq-agent-dev" && lib.hasSuffix "/bin/torq-agent-service dev claude-code" sc.ExecStart
        && sc.ProtectHome == true && sc.PrivateTmp && sc.NoNewPrivileges && sc.CapabilityBoundingSet == "" && sc.AmbientCapabilities == ""
        && sc.ProtectSystem == "strict" && sc.ReadWritePaths == [ "/var/lib/torq-agent-dev" ] && sc.InaccessiblePaths == [ "-/run/credentials" ]
        && sc.StateDirectory == "torq-agent-memory/dev" && sc.RestrictNamespaces && sc.ProtectControlGroups
        && !(sc ? LoadCredential) && !(sc ? ExecStartPre) && !(sc ? Delegate) && !(sc ? RemoveIPC)
        # Decided, not forgotten (2026-11-15): no MemoryDenyWriteExecute. Claude Code 2.1.316 itself runs with it,
        # Node 24 crashes with it; the engine runs a developer's programs (node and the like), as `torq agent run`
        # does, so it stays off. And a /dev/shm of the unit's own.
        && !(sc ? MemoryDenyWriteExecute) && sc.TemporaryFileSystem == [ "/dev/shm:mode=1777,nosuid,nodev" ]
      then "the unit is the bridge, fenced"
      else throw "agent-assertions: the agent's unit is not as it should be: ${builtins.toJSON sc}")
    # `claude` typed plainly (torq.agent.plainCommand): a command of TorqOS's on every person's PATH, there
    # where Claude Code is installed for the agents and the setting is on (the default), and nowhere else. It
    # is not Claude Code: that stays in the agents' own profiles.
    (let
      chosen = { torq.agent.tools = [ "claude-code" ]; torq.unfree.packages = [ "claude-code" ]; torq.agent.claudeCode = pkgs.hello; };
      plain = c: lib.filter (p: lib.getName p == "claude") c.environment.systemPackages;
      on = plain (configOf chosen);
    in
      if lib.length on == 1 && lib.hasInfix "/bin/torq agent claude \"$@\"" (lib.head on).text
        && plain (configOf { imports = [ chosen ]; torq.agent.plainCommand = false; }) == [ ]
        && plain (configOf { }) == [ ] && plain (configOf { torq.agent.tools = [ "claude-code" ]; }) == [ ]
      then "claude typed plainly: where it is installed and the setting is on"
      else throw "agent-assertions: the plain claude command is not exactly where Claude Code is installed and the setting is on")
    # What the service's sandbox rests on besides: said, where the agent has a service.
    # Each the per-instance assertion's own message, which the template's check (below) does not share.
    (refuses "a delegated agent unit" (withEngine { systemd.services."torq-agent@dev".serviceConfig.Delegate = true; }) "systemd.services.\"torq-agent@dev\": no Delegate or DelegateSubgroup")
    (refuses "an agent unit's subgroup" (withEngine { systemd.services."torq-agent@dev".serviceConfig.DelegateSubgroup = "x"; }) "systemd.services.\"torq-agent@dev\": no Delegate or DelegateSubgroup")
    (refuses "a delegated template" (withEngine { systemd.services."torq-agent@".serviceConfig.Delegate = true; }) "in any torq-agent@ unit")
    # The agent's units say only what TorqOS's module writes (modules/agent.nix, intendedText): another part of
    # the configuration that changes one is refused here, at evaluation, by the unit and the setting. (The units
    # as built are held to the same text: tests/agent-units.nix. The control is "an engine installed", above.)
    (let said = "the agent's unit says only what TorqOS's own module writes"; in lib.concatStringsSep ", " [
      (refuses "the agent's user changed" (withEngine { systemd.services."torq-agent@dev".serviceConfig.User = lib.mkForce "root"; }) "systemd.units.\"torq-agent@dev.service\": ${said}")
      (refuses "the agent's user changed, said" (withEngine { systemd.services."torq-agent@dev".serviceConfig.User = lib.mkForce "root"; }) "User: it says `User=root`, which TorqOS's module does not write; User: it lacks `User=torq-agent-dev`, which TorqOS's module writes")
      (refuses "a command before the agent's" (withEngine { systemd.services."torq-agent@dev".serviceConfig.ExecStartPre = [ "+/bin/sh -c true" ]; }) "ExecStartPre: it says `ExecStartPre=+/bin/sh -c true`")
      (refuses "the agent's sandbox changed" (withEngine { systemd.services."torq-agent@dev".serviceConfig.ProtectSystem = lib.mkForce "full"; }) "ProtectSystem: it says `ProtectSystem=full`")
      (refuses "the agent's home opened" (withEngine { systemd.services."torq-agent@dev".serviceConfig.ProtectHome = lib.mkForce "read-only"; }) "ProtectHome: it says `ProtectHome=read-only`")
      (refuses "the socket's mode changed" (withEngine { systemd.sockets."torq-agent@dev".socketConfig.SocketMode = lib.mkForce "0666"; }) "systemd.units.\"torq-agent@dev.socket\": ${said}")
      (refuses "the unit's text forced" (withEngine { systemd.units."torq-agent@dev.service".text = lib.mkForce "[Service]\nExecStart=/bin/sh -c true\n"; }) "ExecStart: it says `ExecStart=/bin/sh -c true`")
    ])
    (refuses "a lingering agent" (withEngine { users.users.torq-agent-dev.linger = true; }) "the agent account has no user manager of its own")
    (refuses "tracing allowed" (withEngine { boot.kernel.sysctl."kernel.yama.ptrace_scope" = lib.mkForce 0; }) "kernel.yama.ptrace_scope")
    # A start that fails does not cost the person their socket (KNOWN-GAPS 6zzo): what waits on the socket is
    # turned away when the service ends, so a failed start is not tried again for the same question, and the
    # unit has no limit of starts that would stop its socket for good. Nothing restarts it by itself. (What
    # this does on a running machine is tests/agent-service.nix's to show.)
    (let
      ok = c: (c.systemd.sockets."torq-agent@dev".socketConfig.FlushPending or null) == true
        && c.systemd.sockets."torq-agent@dev".socketConfig.Accept == false
        && (c.systemd.services."torq-agent@dev".unitConfig.StartLimitIntervalSec or null) == 0
        && !(c.systemd.services."torq-agent@dev".serviceConfig ? Restart);
    in
      if ok (configOf (withEngine { }))
      then "a failed start leaves the socket"
      else throw "agent-assertions: the agent's socket or unit would let failed starts stop the socket for good")
    # The checker's account (ADR-056, modules/broker.nix): what an agent's proposal is evaluated as. It is in
    # its own group and no other, never one the Nix daemon trusts (by its name, by its group, or as everyone),
    # and has no password, no shell, no home and no SSH key: each set by TorqOS's module, and each refused
    # where another part of the configuration says otherwise. (The control is "plain", above, whose failing
    # messages `ours` now takes this one among.)
    (let said = "users.users.torq-check: the account an agent's proposal is checked as (ADR-056) is a system account in its own group and no other, with no password, no shell, no home and no SSH key, and never one the Nix daemon trusts"; in lib.concatStringsSep ", " [
      (let c = configOf { }; u = c.users.users.torq-check; in
        if u.hashedPassword == "!" && lib.hasSuffix "/bin/nologin" u.shell && u.home == "/var/empty" && !u.createHome && u.group == "torq-check"
          && builtins.elem "torq-check" c.nix.settings.allowed-users && lib.unique c.nix.settings.trusted-users == [ "root" ]
        then "the checker's account is locked, allowed and not trusted"
        else throw "agent-assertions: the checker's account is not as TorqOS's module sets it")
      (refuses "the checker trusted by name" { nix.settings.trusted-users = [ "torq-check" ]; } said)
      (refuses "the checker trusted by its group" { nix.settings.trusted-users = [ "@torq-check" ]; } said)
      (refuses "everyone trusted" { nix.settings.trusted-users = [ "*" ]; } said)
      (refuses "the checker in another group" { users.users.torq-check.extraGroups = [ "wheel" ]; } said)
      (refuses "the checker a member of another group" { users.groups.wheel.members = [ "torq-check" ]; } said)
      (refuses "the checker's group changed" { users.users.torq-check.group = lib.mkForce "users"; } said)
      (refuses "the checker a person" { users.users.torq-check = { isSystemUser = lib.mkForce false; isNormalUser = true; }; } said)
      (refuses "a password for the checker" { users.users.torq-check.hashedPassword = lib.mkForce "$6$x$y"; } said)
      (refuses "a password for the checker, in the clear" { users.users.torq-check.password = "x"; } said)
      (refuses "a password file for the checker" { users.users.torq-check.hashedPasswordFile = "/etc/torq-check.pw"; } said)
      (refuses "a first password for the checker" { users.users.torq-check.initialPassword = "x"; } said)
      (refuses "a first password's hash for the checker" { users.users.torq-check.initialHashedPassword = "$6$x$y"; } said)
      (refuses "a shell for the checker" ({ pkgs, ... }: { users.users.torq-check.shell = lib.mkForce pkgs.bash; }) said)
      (refuses "a home for the checker" { users.users.torq-check.home = lib.mkForce "/var/lib/torq-check"; } said)
      (refuses "a home made for the checker" { users.users.torq-check.createHome = lib.mkForce true; } said)
      (refuses "an SSH key for the checker" { users.users.torq-check.openssh.authorizedKeys.keys = [ "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAsQmVpCCIItKDMwqNcgAQIAJH9uMP0VsxjvXJkusubc x" ]; } said)
      (refuses "an SSH key file for the checker" { users.users.torq-check.openssh.authorizedKeys.keyFiles = [ ./removed-settings/nixos/torq.json ]; } said)
    ])
    (refuses "SETENV in sudoers text" ({ config, ... }: {
      security.sudo.extraConfig = "%wheel ALL=(root) NOPASSWD: SETENV: ${config.torq.agent.launcher}/bin/torq-agent-run";
    }) "a rule for torq-agent-run with SETENV")
  ];
in
pkgs.runCommand "torq-agent-assertions" { } ''
  echo ${lib.escapeShellArg (lib.concatStringsSep "\n" cases)} > $out
''
