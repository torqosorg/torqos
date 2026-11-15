# The agents' accounts (ADR-041, ADR-051 3): one per person, `torq-agent-<person>` — the only accounts
# whatever arrives from is an agent (ADR-040: the broker names its caller from the kernel's uid, and
# its map, /etc/torq/broker.json, says which person each agent serves). An agent proposes, and only
# what `torq.agent.mayPropose` lists and its own person's packages; a person applies. Each is made for
# TorqOS's own agent (M5) and for the AI tools people already use — Claude Code first — run as it
# rather than as the person.
{ config, lib, pkgs, utils, ... }:

let
  cfg = config.torq.agent;
  agentOf = person: "torq-agent-${person}";
  # Each agent account, and the person it serves: the broker's map (broker.json), and the accounts below.
  # (Only names that fit: one too long is the assertion's to say, below, not NixOS's abort.)
  # The old loop's second account for each person (ADR-051 3, amended 2026-11-11). Nothing runs as it any more
  # (ADR-055: the agent's service runs the person's engine, as the agent account); it is still made where doctor
  # mode is on, for the broker's rule about it, and goes with both (docs/HARNESS-AS-A-SYSTEM.md, phase 2).
  residentOf = person: "torq-resident-${person}";
  fits = p: lib.stringLength (residentOf p) <= 31;
  # The launcher's sudo rules that would let its caller set the environment (SETENV), as text or as rules.
  isLauncher = c: lib.hasInfix "torq-agent-run" (if builtins.isString c then c else c.command);
  launcherSetenv = lib.filter (c: isLauncher c && !(builtins.isString c) && builtins.elem "SETENV" c.options) (lib.concatMap (r: r.commands) config.security.sudo.extraRules)
    ++ lib.filter (l: lib.hasInfix "torq-agent-run" l && lib.hasInfix "SETENV" l) (lib.splitString "\n" config.security.sudo.extraConfig);
  personsLikeAgents = lib.attrNames (lib.filterAttrs (n: u: u.isNormalUser && (lib.hasPrefix "torq-agent" n || lib.hasPrefix "torq-resident" n)) config.users.users);
  agents = lib.listToAttrs (map (p: lib.nameValuePair (agentOf p) p) (lib.filter fits cfg.people));
  # Where the agent's service is (its socket, its unit): where an engine is chosen (ADR-055 1).
  bridged = cfg.engine != null;
  # Where the old loop's accounts are still made, and the broker's map of them: where doctor mode is on.
  resident = config.torq.agent.doctor.enable;
  # Any torq-agent@ unit this configuration writes (an instance or the template) whose text delegates its control
  # group. A unit file a package brings (systemd.packages) is not in systemd.units, so not seen here: it is a file
  # inside a built package, which evaluation cannot read. Those are read in the units as built (unitsCheck).
  delegatedUnits = lib.attrNames (lib.filterAttrs (n: u: lib.hasPrefix "torq-agent@" n && lib.hasInfix "Delegate" (if u.text or null == null then "" else u.text)) config.systemd.units);
  unitsCheck = import ./agent-units-check.nix { inherit pkgs; };
  # The agent's units as this module writes them (below), each with the one file NixOS makes of it: what the
  # check on the units as built holds the finished directory to. Nothing else there may be named as they are,
  # and none may have a drop-in.
  ownUnits = lib.concatMap (person: [ "torq-agent@${person}.service" "torq-agent@${person}.socket" ]) (lib.attrValues agents);
  expectedUnits = lib.genAttrs ownUnits (name: "${config.systemd.units.${name}.unit}/${name}");
  # What each of those units must say, word for word (2026-11-14, KNOWN-GAPS 6zzo): the text NixOS makes of
  # this module's own values (ownSockets, ownServices, below) and of nothing else. Why this way. Another module
  # can change the agent's unit through the module system (`systemd.services."torq-agent@<person>".serviceConfig
  # .User = lib.mkForce "root"`, a new ExecStartPre, a credential, a sandbox line taken back) and the result is
  # still "the file the configuration wrote", which is all the rule on whose the files are can see. No list of
  # settings that matter has an end, so none is kept here either: the whole text must be the same. The values
  # are put through nixpkgs' own types and its own writer of unit text (`utils.systemdUtils`), so nothing of
  # NixOS's rendering is copied here, and through the types as nixpkgs declares them, not `options.systemd
  # .services.type`: a module that adds settings to every service by extending that option's type is then seen
  # as a difference too. The same text is held against the finished file in the units as built (unitsCheck),
  # which is the one that cannot be gone round (`systemd.units.<name>.text` or `.unit` forced directly); the
  # assertion below says it earlier, at `commit check`, with the option's name.
  # What still comes from the rest of the configuration, by design: what NixOS gives every service
  # (`systemd.globalEnvironment`: LOCALE_ARCHIVE, TZDIR; the default PATH's packages), and the programs' own
  # store paths.
  alone = kind: type: values: (lib.mergeDefinitions [ "systemd" kind ] type [ { file = "TorqOS's modules/agent.nix"; value = values; } ]).mergedValue;
  intendedText =
    lib.mapAttrs' (n: v: lib.nameValuePair "${n}.service" (utils.systemdUtils.lib.serviceToUnit v).text) (alone "services" utils.systemdUtils.types.services ownServices)
    // lib.mapAttrs' (n: v: lib.nameValuePair "${n}.socket" (utils.systemdUtils.lib.socketToUnit v).text) (alone "sockets" utils.systemdUtils.types.sockets ownSockets);
  intendedUnits = lib.genAttrs ownUnits (name: intendedText.${name});
  # How a unit's text differs from what this module writes, said by the setting (the word before `=`).
  textDifferences = name:
    let
      lines = t: lib.filter (l: l != "") (lib.splitString "\n" (if t == null then "" else t));
      want = lines intendedUnits.${name};
      got = lines (config.systemd.units.${name}.text or null);
      setting = l: lib.head (lib.splitString "=" l);
      says = lib.subtractLists want got;
      lacks = lib.subtractLists got want;
    in
      map (l: "${setting l}: it says `${l}`, which TorqOS's module does not write") says
      ++ map (l: "${setting l}: it lacks `${l}`, which TorqOS's module writes") lacks
      ++ lib.optional (says == [ ] && lacks == [ ] && got != want) "its lines are in another order, or said more than once";
  # Only the person may connect: the socket theirs, 0600, in a directory root's.
  ownSockets = lib.mapAttrs' (agent: person: lib.nameValuePair "torq-agent@${person}" {
    description = "${person}'s TorqOS agent: its socket";
    wantedBy = [ "sockets.target" ];
    socketConfig = {
      ListenStream = "/run/torq-agent/${person}.sock";
      SocketUser = person;
      SocketGroup = "root";
      SocketMode = "0600";
      DirectoryMode = "0755";
      Accept = false;
      RemoveOnStop = true;
      # A start that fails must not cost the person their socket. The
      # connection that asked for the start stays waiting on the socket when the service fails, and systemd,
      # listening again, would start the service for it again at once, over and over, until a limit stopped
      # the socket for good (failed, and its file removed): then nothing answers until an administrator
      # restarts the socket, whatever was wrong long put right. With this, systemd turns away what is waiting each time
      # the service ends, so one question is one start, and the socket stays. (systemd 260.5,
      # src/core/socket.c: socket_trigger_notify, socket_enter_listening. Together with the service's
      # StartLimitIntervalSec=0, below.) The cost: a question that arrives in the instant the service ends is
      # turned away, and is asked again.
      FlushPending = true;
    };
  }) agents;
  ownServices = lib.mapAttrs' (agent: person: lib.nameValuePair "torq-agent@${person}" {
    description = "${person}'s TorqOS agent (proposes; a person applies)";
    requires = [ "torq-agent@${person}.socket" ];
    after = [ "torq-agent@${person}.socket" "torq-broker.socket" ];
    # No limit on how often it may be started. systemd's own (5 starts in 10 s) would otherwise be reached by a
    # person asking a few times while its start fails, and a socket whose service has reached it is stopped
    # for good ("service-start-limit-hit"): what was wrong put right, the agent would still not answer until an
    # administrator restarted the socket. Nothing restarts this unit by itself (no Restart=); only a connection
    # starts it, one start for each (the socket's FlushPending), and only its own person can connect. How often
    # that may be is the socket's to limit, by systemd's defaults, left as they are: more than 15 in 2 s are
    # slowed (PollLimitBurst), and behind that 20 in 2 s would still stop the socket (TriggerLimitBurst), which
    # asking by hand does not reach.
    unitConfig.StartLimitIntervalSec = 0;
    serviceConfig = {
      # The bridge (agent/src/service): it runs the engine named here, hands it each question, and passes on
      # what it says. `bridged` (below) says this unit is only where an engine is chosen.
      ExecStart = "${config.torq.agent.doctor.package}/bin/torq-agent-service ${person} ${toString cfg.engine}";
      # The listening socket as standard input (Accept=no): the program takes it from there.
      StandardInput = "socket";
      StandardOutput = "journal";
      StandardError = "journal";
      # The person's one agent account (ADR-055 2): the engine runs as it, as every program `torq agent run`
      # starts does, and so does the bridge that starts it. The broker takes whatever arrives from this
      # account for a coding tool's, tainted: each proposal waits for the person (broker/src/socket.rs).
      User = agent;
      Group = agent;
      # What the service keeps (the conversation's name at the engine, its count: agent/src/service/kept.rs),
      # 0700, outside the agent's home, hidden from `torq agent run` (modules/agent-run.sh).
      WorkingDirectory = "/var/lib/torq-agent-memory/${person}";
      StateDirectory = "torq-agent-memory/${person}";
      StateDirectoryMode = "0700";
      # The agent's own home: the engine's home and its working directory (its settings, its sign-in, its
      # conversations), and the one other place it may write (the system is read-only, below).
      ReadWritePaths = [ "/var/lib/${agent}" ];
      # What systemd gives other units as credentials: hidden, as `torq agent run` hides it.
      InaccessiblePaths = [ "-/run/credentials" ];
      # The launcher's sandbox: every home (and /root, and the people's runtime directories in
      # /run/user) out of reach, /tmp its own, no privilege at all.
      ProtectHome = true;
      PrivateTmp = true;
      NoNewPrivileges = true;
      CapabilityBoundingSet = "";
      AmbientCapabilities = "";
      # And more than the launcher's: the system read-only but its home and what it keeps; no other user's
      # processes seen; the kernel's settings, modules, logs and clock, and the control groups, untouchable (no
      # Delegate: nothing the engine starts can leave this unit's group, whose cgroup.procs is root's, so the
      # service ends all of it when the engine is stopped).
      ProtectSystem = "strict";
      ProtectProc = "invisible";
      ProtectKernelTunables = true;
      ProtectKernelModules = true;
      ProtectKernelLogs = true;
      ProtectControlGroups = true;
      ProtectClock = true;
      ProtectHostname = true;
      RestrictNamespaces = true;
      RestrictRealtime = true;
      RestrictSUIDSGID = true;
      LockPersonality = true;
      # Not MemoryDenyWriteExecute: an engine is a JavaScript program (Claude Code), whose runtime writes the
      # code it then runs; with it the engine does not start. `torq agent run` never had it.
      # Its own IPC namespace: what the engine makes there goes with the unit. Not RemoveIPC: that takes away
      # the account's objects on the whole machine when this unit stops, and the account is also the one a
      # `torq agent run` session at a terminal runs as.
      PrivateIPC = true;
      KeyringMode = "private";
      UMask = "0077";
      # Unix sockets (the broker), the network (the engine's own service), and netlink (a program reads the
      # machine's interfaces through it: torq-mcp's `ip` and `iw`, the engine's runtime).
      RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" "AF_NETLINK" ];
      SystemCallFilter = [ "@system-service" ];
      SystemCallErrorNumber = "EPERM";
      SystemCallArchitectures = "native";
      # Not PrivateUsers: in a user namespace of its own, SO_PEERCRED would name every caller nobody.
      # Not PrivateDevices: torq-mcp's rfkill and lsblk read devices.
    };
  }) agents;
  residents = lib.optionalAttrs resident (lib.listToAttrs (map (p: lib.nameValuePair (residentOf p) p) (lib.filter fits cfg.people)));
  # Claude Code for the agents (ADR-051 5, `torq agent tools`): the version dev mode pins (modules/dev.nix), unfree,
  # so only where `torq unfree` allows it — else the assertion below says so, and nothing unfree is evaluated.
  claudeChosen = builtins.elem "claude-code" cfg.tools;
  claudeAllowed = config.torq.unfree.all || builtins.elem "claude-code" config.torq.unfree.packages;
  claudeCode = pkgs.claude-code.override { manifest = lib.importJSON ./claude-code-manifest.json; };
  # `torq agent run` (ADR-041, 5): a person starts a program as their own agent. This runs as root
  # (through sudo), hides every home and the people's runtime directories, gives a private /tmp,
  # binds in only the directory shared with it, and drops to the caller's agent for good.
  agentRun = pkgs.writeShellApplication {
    name = "torq-agent-run";
    # systemctl and sed: holding the agent's service still while a directory is shared (--hold-frozen).
    runtimeInputs = [ pkgs.bubblewrap pkgs.coreutils pkgs.util-linux pkgs.bash pkgs.jq pkgs.getent pkgs.gnused config.systemd.package ];
    text = builtins.readFile ./agent-run.sh;
  };
in
{
  imports = [
    # Removed, not renamed: it said whose packages the one agent might propose; `people` says who has an
    # agent of their own, which proposes only that person's. The same list would mean something else.
    (lib.mkRemovedOptionModule [ "torq" "agent" "mayPropose" "users" ] ''
      Each person now has an agent account of their own (ADR-051 3), which proposes packages only for
      that person: list the people who get one in torq.agent.people (the owner, by default). No agent
      proposes another person's packages any more.'')
    # Removed, not renamed: the service was TorqOS's own loop, on wherever doctor mode was. It is now the
    # bridge to the person's engine, and is there exactly where an engine is chosen (ADR-055).
    (lib.mkRemovedOptionModule [ "torq" "agent" "service" "enable" ] ''
      The agent's service is there where an engine is chosen: set torq.agent.engine (`set torq agent engine
      claude-code`) to have it, and leave it unset to have none.'')
  ];

  options.torq.agent.launcher = lib.mkOption {
    type = lib.types.package;
    internal = true;
    readOnly = true;
    default = agentRun;
    description = "The launcher of `torq agent run` (strict mode names it in its sudo rule).";
  };

  options.torq.agent.people = lib.mkOption {
    # A login name's shape, as the broker's (policy.rs, user_name_ok).
    type = lib.types.listOf (lib.types.strMatching "[a-z_][a-z0-9_-]*");
    default = lib.optional (config.torq.user != null) config.torq.user;
    defaultText = lib.literalExpression "[ config.torq.user ]";
    example = [ "alice" "bob" ];
    description = ''
      The people who each get an agent account of their own, `torq-agent-<name>` (ADR-051 3): no
      password, no login, no group but its own, its home /var/lib/torq-agent-<name> its own. Each
      person's agent proposes packages only for that person (`users <name> packages`), and settings
      only as `mayPropose.settings` allows; that person's trust covers it and no other. The owner, by
      default. Each must be a person of this machine (a normal user), with a name of at most 17
      characters (NixOS takes an account's and a group's name of at most 31).
    '';
  };

  options.torq.agent.tools = lib.mkOption {
    type = lib.types.listOf (lib.types.enum [ "claude-code" ]);
    default = [ ];
    example = [ "claude-code" ];
    description = ''
      The coding tools installed for each person's agent account (ADR-051 5): `claude-code`, Claude Code — in
      the agent accounts' own profiles, on no person's PATH. It runs as the person's agent, never as the person:
      `torq agent run --share <folder> -- claude`, its sign-in (a subscription's too) kept in the agent's home.
      At each start the launcher links TorqOS's MCP server (torq-mcp) and skill into its settings there; what it
      proposes is tainted and waits for the person (ADR-051 7). Claude Code is unfree: allow it first
      (`set torq unfree packages claude-code`).
    '';
  };

  options.torq.agent.engine = lib.mkOption {
    type = lib.types.nullOr (lib.types.enum [ "claude-code" ]);
    default = null;
    example = "claude-code";
    description = ''
      The coding tool that is this machine's agent: `claude-code`, Claude Code. Every place in TorqOS that
      offers AI (the agent's window and panel applet, `torq ask` at a terminal) then reaches it. It runs as
      each person's own agent account, in the same sandbox as `torq agent run`: it sees its own home, not
      yours, and can only propose a change to the system, which you approve. Install it for the agents first
      (`set torq agent tools claude-code`), and sign in once: `torq agent run -- claude`, then /login. Not
      set, the default: the machine has no agent to ask.
    '';
  };

  options.torq.agent.claudeCode = lib.mkOption {
    type = lib.types.package;
    internal = true;
    default = claudeCode;
    defaultText = lib.literalExpression "pkgs.claude-code, the version modules/claude-code-manifest.json pins";
    description = "Claude Code, as `torq agent tools` installs it for the agents (a VM test puts a stand-in here: the real one is unfree, and large).";
  };

  options.torq.agent.residents = lib.mkOption {
    type = lib.types.attrsOf lib.types.str;
    internal = true;
    readOnly = true;
    default = residents;
    description = "The old loop's second account for each person and the person it serves (the broker's map, /etc/torq/broker.json; ADR-051 3 amended 2026-11-11). Only where doctor mode is on; nothing runs as it since ADR-055.";
  };

  options.torq.agent.agents = lib.mkOption {
    type = lib.types.attrsOf lib.types.str;
    internal = true;
    readOnly = true;
    default = agents;
    description = "Each agent account and the person it serves (the broker's map, /etc/torq/broker.json).";
  };

  options.torq.agent.mayPropose = {
    settings = lib.mkOption {
      type = lib.types.listOf (lib.types.strMatching "[A-Za-z0-9_-]+(\\.[A-Za-z0-9_-]+)*");
      default = [ ];
      example = [ "services.openssh" "environment.systemPackages" "system.host" ];
      description = ''
        The settings every agent may propose changes in (the machine's list, the same for each person's
        agent), as subtrees of the tree (dotted paths): every
        setting a proposal of the agent's changes must lie inside one of them, or the broker refuses
        it, and ledgers the refusal. None, by default: the agent proposes packages and nothing else.
        `torq.*` is never the agent's, listed or not (its own limits are there), and nor are config
        files, `flake.nix` or the lock.

        `system.<name>` means both torqcli's own settings (`system host-name`, `system packages`:
        `system.host`, `system.packages`) and NixOS's `system.*` options of that name — list the
        narrowest one. A bare `system` is refused: it would allow `system.activationScripts`, code
        run as root.
      '';
    };
  };

  config = lib.mkMerge [ {
    assertions = [{
      assertion = lib.all (s: lib.head (lib.splitString "." s) != "torq") cfg.mayPropose.settings;
      message = "torq.agent.mayPropose.settings: torq.* is never the agent's (its own limits are there).";
    }
    {
      assertion = !(builtins.elem "system" cfg.mayPropose.settings);
      message = "torq.agent.mayPropose.settings: a bare `system` would let the agent propose system.activationScripts (code run as root); list the setting itself (system.host, …).";
    }]
    # Each person's agent is that person's: a person of this machine, an account name Linux takes
    # (at most 31 characters, as NixOS takes a group's) and the broker does (policy.rs), once each, and never itself an agent's name.
    ++ lib.concatMap (p: [
      {
        assertion = config.users.users ? ${p} && config.users.users.${p}.isNormalUser;
        message = "torq.agent.people: ${p} is not a person on this machine (a normal user): an agent account is a person's.";
      }
      {
        assertion = fits p;
        message = "torq.agent.people: ${residentOf p} (its resident agent's account) is longer than an account's and its group's name may be (31 characters on NixOS): ${p} is too long a name for an agent of its own (at most 17 characters).";
      }
      {
        assertion = !(lib.hasPrefix "torq-agent" p);
        message = "torq.agent.people: ${p} is named as an agent account is; a person's name never begins with torq-agent.";
      }
    ]) cfg.people
    ++ [{
      assertion = lib.unique cfg.people == cfg.people;
      message = "torq.agent.people: a person is named twice.";
    }]
    ++ [{
      # Whose agent the launcher runs is sudo's SUDO_UID (modules/agent-run.sh). A rule that lets the
      # caller set the environment lets them set that too (`sudo SUDO_UID=<another's> …` is taken as
      # given, and `Defaults !setenv` does not undo a rule's SETENV): never on the launcher's rules.
      assertion = launcherSetenv == [ ];
      message = "security.sudo: a rule for torq-agent-run with SETENV would let its caller say whose agent it runs (SUDO_UID); it never has one.";
    }
    {
      # No person is named as an agent account is, whether they have an agent or not: torqcli tells an
      # agent by its name (cli/src/agent.rs, is_agent), and would take such a person for one.
      assertion = personsLikeAgents == [ ];
      message = "users.users: ${lib.concatStringsSep ", " personsLikeAgents} is a person named as an agent account is (torq-agent…): a person's name never begins so.";
    }
    {
      # ADR-039: nothing unfree in an image, and nothing unfree unasked. Claude Code is unfree in nixpkgs.
      assertion = !claudeChosen || claudeAllowed;
      message = "torq.agent.tools: claude-code is unfree (nixpkgs' licence), and nothing unfree is installed unless allowed: allow it first with `set torq unfree packages claude-code` (or `set torq unfree all true`), then `set torq agent tools claude-code`.";
    }
    {
      # The engine is one of the coding tools installed for the agents: without its program there is nothing to run.
      assertion = cfg.engine == null || builtins.elem cfg.engine cfg.tools;
      message = "torq.agent.engine: ${toString cfg.engine} is not installed for the agents. Install it first: `set torq agent tools ${toString cfg.engine}`, then `set torq agent engine ${toString cfg.engine}`.";
    }];

    # Static system users: the broker names its caller from /etc/passwd, where a DynamicUser is not.
    # Each: no password, no login shell (no SSH, no console), no group but its own: no sudo, no
    # devices. Its home holds what a tool keeps (Claude Code's ~/.claude, its credentials) — never the
    # person's, and never another agent's. What it changes in the system, it proposes through the
    # broker's socket, as torqcli does.
    users.users = lib.mapAttrs (agent: person: {
      isSystemUser = true;
      group = agent;
      description = "${person}'s TorqOS agent (proposes; a person applies)";
      home = "/var/lib/${agent}";
      createHome = true;
      homeMode = "700";
      hashedPassword = "!";
      # The coding tools chosen (ADR-051 5, `torq agent tools`), in the agent's own profile
      # (/etc/profiles/per-user/torq-agent-<person>, on the launcher's PATH): on no person's PATH.
      packages = lib.optional (claudeChosen && claudeAllowed) cfg.claudeCode;
    }) agents;
    users.groups = lib.mapAttrs (_: _: { }) agents;

    # A person (an administrator) may start a program as their own agent, only through the launcher:
    # it binds the one shared directory as root, then drops to the caller's agent for good before
    # running anything (modules/agent-run.sh). No password: administrators have root already, and the
    # program runs with less than they have.
    environment.systemPackages = [ agentRun pkgs.acl ]
      # torq-mcp, where a coding tool run as the agent finds it (/run/current-system/sw/bin/torq-mcp).
      ++ lib.optional (cfg.tools != [ ]) config.torq.agent.doctor.package;
    # The coding tools chosen (the launcher links TorqOS into them at each start; the Agent page shows them),
    # and TorqOS's skill, linked into each (ADR-051 7): advice, which the tool may rewrite.
    # And the engine among them, if one is chosen (ADR-055 1): what the person's programs say answers them.
    environment.etc."torq/agent-tools.json".text = builtins.toJSON { inherit (cfg) tools engine; };
    environment.etc."torq/skills/torqos/SKILL.md".source = ./agent-skill.md;
    # Where the launcher binds a shared directory first, as root, before checking it (root's alone).
    systemd.tmpfiles.settings."10-torq-agent"."/run/torq-agent-stage".d = { mode = "0700"; user = "root"; group = "root"; };
    security.sudo.extraRules = [{
      groups = [ "wheel" ];
      runAs = "root";
      commands = [{ command = "${agentRun}/bin/torq-agent-run"; options = [ "NOPASSWD" ]; }];
    }];
    # The shared directory comes as a descriptor torqcli opened, as the person — the agent has no
    # way into their home: sudo keeps that descriptor open for the launcher (`-C`).
    security.sudo.extraConfig = ''
      Defaults!${agentRun}/bin/torq-agent-run closefrom_override, env_reset
    '';
  }

  # The old loop's second accounts (ADR-051 3, amended 2026-11-11), where doctor mode is on: hidden — no
  # password, no login, no home, no group but its own. Nothing runs as one since ADR-055; the broker still
  # counts it as the person's agent (broker.json's `residents`) until its rule goes (phase 2).
  (lib.mkIf resident {
    users.users = lib.mapAttrs (account: person: {
      isSystemUser = true;
      group = account;
      description = "${person}'s TorqOS agent, resident (the old loop's account; proposes, a person applies)";
      home = "/var/empty";
      createHome = false;
      hashedPassword = "!";
    }) residents;
    users.groups = lib.mapAttrs (_: _: { }) residents;
  })

  # Each person's agent, as a service (ADR-051 3, ADR-055 3): `torq-agent@<person>.service`, started by its
  # socket the first time the person asks (`torq ask`, the agent's window), so it costs nothing until then.
  # It is the bridge to the engine: it runs the engine as one long-lived process for the person's
  # conversation, as the agent, in the sandbox below — at least the launcher's (every home hidden, the
  # people's runtime directories hidden, its own /tmp, no privilege, its home its own) and systemd's
  # hardening besides. Only where an engine is chosen.
  (lib.mkIf bridged {
    # What the service's sandbox rests on besides the unit's own text, each said: the unit's control group is
    # root's to fill and nothing in it can leave (nothing delegated, in any torq-agent@ unit this configuration
    # writes — an instance or the template), so stopping the engine ends all of it; no agent account has a
    # user manager to make units with; a same-uid program may trace only its own descendants (the engine
    # cannot trace the bridge that started it, nor a `torq agent run` session).
    assertions = lib.concatLists (lib.mapAttrsToList (agent: person:
      let sc = config.systemd.services."torq-agent@${person}".serviceConfig; in [
        {
          assertion = !(sc ? Delegate) && !(sc ? DelegateSubgroup);
          message = "systemd.services.\"torq-agent@${person}\": no Delegate or DelegateSubgroup — nothing ${person}'s agent starts may leave this unit's control group, which only root may move a process into or out of: stopping the agent ends all of it.";
        }
        {
          assertion = (config.users.users.${agent}.linger or null) != true;
          message = "users.users.${agent}.linger: the agent account has no user manager of its own (its units would be its own to make, outside its service's control group and its sandbox).";
        }
      ]) agents)
      # Each of the agent's units says what this module writes and nothing else (intendedText, above): who it
      # runs as, what it runs, its credentials, its sandbox, its socket. Said here for `commit check`; the units
      # as built are held to the same text (unitsCheck).
      ++ map (name: {
        assertion = (config.systemd.units.${name}.text or null) == intendedUnits.${name};
        message = "systemd.units.\"${name}\": the agent's unit says only what TorqOS's own module writes (who the agent runs as, what it runs, its credentials, its sandbox, its socket); another part of this configuration changes it. ${lib.concatStringsSep "; " (textDifferences name)}.";
      }) ownUnits
      ++ [{
        assertion = delegatedUnits == [ ];
        message = "systemd.units ${lib.concatStringsSep ", " delegatedUnits}: no Delegate or DelegateSubgroup in any torq-agent@ unit, an instance or the template (the unit's control group is root's to fill: stopping the agent ends all of it).";
      }
      {
        assertion = (config.boot.kernel.sysctl."kernel.yama.ptrace_scope" or 0) >= 1;
        message = "boot.kernel.sysctl.\"kernel.yama.ptrace_scope\": at least 1 where the agent has a service: a program of the agent's may trace only its own descendants.";
      }];
    # The same "nothing delegated", for what the assertions above cannot see: a unit file or drop-in a package
    # brings (systemd.packages). Read in the directory /etc/systemd/system is made from, where the packages'
    # files and this configuration's are together; the system's build depends on it, so a system that fails it
    # is never built or switched to (modules/agent-units-check.nix). `commit check` only evaluates: it is the
    # build that refuses.
    # And, since 2026-11-14, whose the files are: the agent's units are there as exactly the files this module
    # writes (expectedUnits), with no drop-in and no other unit of their names, so nothing a package brings can
    # change who the agent runs as, what it runs, its credentials or its sandbox. And each of those files says,
    # word for word, what this module writes (intendedUnits), so nothing in the configuration can either.
    system.checks = [ (unitsCheck { units = config.environment.etc."systemd/system".source; expected = expectedUnits; intended = intendedUnits; }) ];
    # Only the person may connect (the socket theirs, 0600, in a directory root's) — and the service
    # answers only them, by the kernel's word (SO_PEERCRED), root included.
    # A linger file is how an account gets a user manager; the assertion above reads the configuration.
    systemd.sockets = ownSockets;
    systemd.services = ownServices;
  })
  ];
}
