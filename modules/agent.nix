# The agents' accounts (ADR-041, ADR-051 3, ADR-055): one per person, `torq-agent-<person>` — the only accounts
# whatever arrives from is an agent (ADR-040: the broker names its caller from the kernel's uid, and
# its map, /etc/torq/broker.json, says which person each agent serves). An agent proposes, and only
# what `torq.agent.mayPropose` lists and its own person's packages; a person applies.
#
# What runs as the account is the person's coding tool (ADR-055): Claude Code, the one TorqOS knows so far.
# Three ways, one fence:
# - the agent's service (`torq-agent@<person>`, where `torq.agent.engine` names an engine): the bridge that
#   runs the engine for the desktop's AI and `torq ask`;
# - `torq agent run [--share <folder>] -- <program>`: any program at a terminal, as the agent;
# - `claude`, typed plainly (`torq.agent.plainCommand`): Claude Code as the agent, in the folder the person is in.
# TorqOS's tools for it (`torq-mcp`: reading the machine, proposing a change) are installed with the coding tools;
# whether they may read the system journal is `torq.agent.journal`. TorqOS has no agent loop or model of its
# own (removed in phase 2 of ADR-055, with the settings under `torq agent doctor` and `torq agent model`).
#
# All of `torq.*` is never the agent's to propose (ADR-041): it cannot change its own engine, tools or journal
# access.
{ config, lib, pkgs, utils, ... }:

let
  cfg = config.torq.agent;
  # The model proxy and the fence around the agents' network (ADR-057, modules/model-proxy.nix).
  proxy = config.torq.modelProxy;
  agentOf = person: "torq-agent-${person}";
  # Each agent account, and the person it serves: the broker's map (broker.json), and the accounts below.
  # (Only names that fit: one too long is the assertion's to say, below, not NixOS's abort.)
  # The account made for a person must fit (31 characters, as NixOS takes a group's): `torq-agent-` and a name
  # of at most 20.
  nameLimit = 20;
  fits = p: lib.stringLength (agentOf p) <= 31;
  # The launcher's sudo rules that would let its caller set the environment (SETENV), as text or as rules.
  isLauncher = c: lib.hasInfix "torq-agent-run" (if builtins.isString c then c else c.command);
  launcherSetenv = lib.filter (c: isLauncher c && !(builtins.isString c) && builtins.elem "SETENV" c.options) (lib.concatMap (r: r.commands) config.security.sudo.extraRules)
    ++ lib.filter (l: lib.hasInfix "torq-agent-run" l && lib.hasInfix "SETENV" l) (lib.splitString "\n" config.security.sudo.extraConfig);
  personsLikeAgents = lib.attrNames (lib.filterAttrs (n: u: u.isNormalUser && lib.hasPrefix "torq-agent" n) config.users.users);
  agents = lib.listToAttrs (map (p: lib.nameValuePair (agentOf p) p) (lib.filter fits cfg.people));
  # Where the agent's service is (its socket, its unit): where an engine is chosen (ADR-055 1).
  bridged = cfg.engine != null;
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
    # Never without the fence that closes the account's network (ADR-057, modules/model-proxy.nix): the unit
    # is not started where it is not up, and is stopped where it goes. The model proxy is wanted, not
    # required: without it the engine reaches no model and says so.
    # And never before the model proxy's port is held for it (its socket), whoever else would listen there.
    requires = [ "torq-agent@${person}.socket" proxy.fence ];
    bindsTo = [ "torq-model-proxy.socket" ];
    wants = [ "torq-model-proxy.service" ];
    after = [ "torq-agent@${person}.socket" "torq-broker.socket" proxy.fence "torq-model-proxy.socket" "torq-model-proxy.service" ];
    # Where the engine's model is and how a site is reached: TorqOS's model proxy (ADR-057). Said here for
    # every program the engine starts; for Claude Code itself the settings below say the same, and it cannot
    # override those.
    environment = proxy.agentEnvironment;
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
      # Started through torq-agent-scope (agent/src/scope), as everything under `torq agent run` is: the socket
      # families' filter (modules/agent-socket-families.c: socketpair of unix alone, which systemd's own filter
      # below does not judge), then Landlock's scopes: no abstract unix socket and no signal but its own. On a
      # kernel without those (before Linux 6.12) it starts nothing, and says so.
      ExecStart = "${cfg.package}/bin/torq-agent-scope --families ${socketFamilies} -- ${cfg.package}/bin/torq-agent-service ${person} ${toString cfg.engine}";
      # The fence by the kernel's word, not by its unit's (which stays active through a `nft flush ruleset`): its
      # table loaded and naming this account, or nothing starts. As root (`+`: reading the packet filter needs it),
      # before anything runs as the agent.
      ExecStartPre = "+${proxy.fenceCheck}/bin/torq-agent-fence-check ${agent}";
      # The listening socket as standard input (Accept=no): the program takes it from there.
      StandardInput = "socket";
      StandardOutput = "journal";
      StandardError = "journal";
      # The person's one agent account (ADR-055 2): the engine runs as it, as every program `torq agent run`
      # starts does, and so does the bridge that starts it. Whatever this account proposes at the broker
      # waits for the person, who reads and approves each proposal (broker/src/session.rs).
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
      # And the services that look a name up for anyone who asks (nscd, systemd-resolved): a name looked up is
      # a message sent, and the agent's network is closed, DNS included (ADR-057). The account's own name is
      # still found, in /etc/passwd.
      # And avahi's own socket (mDNS: a name looked up on the local network, for anyone who asks; its bus name is
      # refused to the agents too, modules/model-proxy.nix).
      InaccessiblePaths = [ "-/run/credentials" "-/run/nscd" "-/run/systemd/resolve" "-/run/avahi-daemon" ];
      # Claude Code's settings for the agents, where Claude Code looks for the settings an account cannot
      # override (/etc/claude-code): shown there in this unit only, never to a person's own Claude Code.
      BindReadOnlyPaths = [ "/etc/torq/claude-code:/etc/claude-code" ];
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
      # Not MemoryDenyWriteExecute. Claude Code 2.1.316 itself runs with it (tried by hand, 2026-11-15); Node 24
      # does not (it crashes at its start: its runtime writes the code it then runs). It stays off because the
      # engine runs a developer's programs (node and the like), which must work here as they do under `torq
      # agent run`, which never had it. (tests/agent-assertions.nix holds the decision.)
      # Its own IPC namespace: what the engine makes there goes with the unit. Not RemoveIPC: that takes away
      # the account's objects on the whole machine when this unit stops, and the account is also the one a
      # `torq agent run` session at a terminal runs as.
      PrivateIPC = true;
      # And a /dev/shm of its own, empty at each start and gone with the unit: the machine's is one every
      # account can write to and read from, so a file another account left there is not the engine's to find,
      # and what the engine leaves there is no one else's (PrivateIPC covers the message queues, not this).
      TemporaryFileSystem = [ "/dev/shm:mode=1777,nosuid,nodev" ];
      KeyringMode = "private";
      UMask = "0077";
      # Unix sockets (the broker), the network (the engine's own service), and netlink (a program reads the
      # machine's interfaces through it: torq-mcp's `ip` and `iw`, the engine's runtime).
      RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" "AF_NETLINK" ];
      # Not io_uring: a socket it makes is of any family (its own operation, not the `socket` call that
      # RestrictAddressFamilies filters), a way past the fence (the controller's review, 2026-11-17). A program
      # refused it does what it does where the kernel has none.
      # And the three calls torq-agent-scope makes to keep itself to its own (seccomp, landlock_create_ruleset,
      # landlock_restrict_self; not in @system-service): each only ever takes away from the process that makes it.
      SystemCallFilter = [ "@system-service" "seccomp landlock_create_ruleset landlock_restrict_self" "~io_uring_setup io_uring_enter io_uring_register" ];
      SystemCallErrorNumber = "EPERM";
      SystemCallArchitectures = "native";
      # Not PrivateUsers: in a user namespace of its own, SO_PEERCRED would name every caller nobody.
      # Not PrivateDevices: torq-mcp's rfkill and lsblk read devices.
    };
  }) agents;
  # Claude Code for the agents (ADR-051 5, `torq agent tools`): the version dev mode pins (modules/dev.nix), unfree,
  # so only where `torq unfree` allows it — else the assertion below says so, and nothing unfree is evaluated.
  claudeChosen = builtins.elem "claude-code" cfg.tools;
  claudeAllowed = config.torq.unfree.all || builtins.elem "claude-code" config.torq.unfree.packages;
  claudeCode = pkgs.claude-code.override { manifest = lib.importJSON ./claude-code-manifest.json; };
  # `claude`, typed plainly (torq.agent.plainCommand): `torq agent claude` (cli/src/agent.rs), a command a person
  # could type themselves. Claude Code as the person's agent, the folder they are in
  # shared with it, every word after it passed on as it is. On every person's PATH, after their own profile
  # (a Claude Code of their own is found first). Inside the sandbox the agent's own profile comes before the
  # system's, so `claude` there is the real one; were it missing, this would be found again and `torq agent
  # run` would refuse it ("an agent's account already"): it cannot start itself over and over.
  plainClaude = pkgs.writeShellScriptBin "claude" ''
    exec ${lib.getExe' config.torq.cli.package "torq"} agent claude "$@"
  '';
  # A setting TorqOS removed, refused where a configuration still sets it. NixOS's mkRemovedOptionModule
  # with words of our own: torqcli shows the first 300 characters of a refusal, so what to type comes first
  # (`delete` works on it in the torqcli of the TorqOS before and of this one), the file it is in is named,
  # and `after` is given what is set there, to say only what applies.
  removed = path: after: { options, ... }:
    let
      opt = lib.getAttrFromPath path options;
      words = lib.concatStringsSep " " path;
    in {
      options = lib.setAttrByPath path (lib.mkOption {
        visible = false;
        apply = _: throw "`${words}` was removed from TorqOS and cannot be used";
      });
      config.assertions = [{
        assertion = !opt.isDefined;
        message = "`${words}` is still set, in ${lib.showFiles opt.files}, and TorqOS no longer has it. "
          + "In torqcli: `configure`, `delete ${words}`, `commit` (with the update, where this is one).\n${after opt.definitions}";
      }];
    };
  # `torq agent run` (ADR-041, 5): a person starts a program as their own agent. This runs as root
  # (through sudo), hides every home and the people's runtime directories, gives a private /tmp,
  # binds in only the directory shared with it, and drops to the caller's agent for good.
  # polkit's rule for the agents (below): every action refused to every agent account, whatever session its
  # process is in and whatever the action's own default. (No person is named as an agent is: the assertion below.)
  polkitRuleFile = "polkit-1/rules.d/00-torq-agents.rules";
  polkitRule = ''
    // TorqOS (modules/agent.nix): an agent's account (torq-agent-<person>) is never the person at the machine.
    polkit.addRule(function(action, subject) {
      if (subject.user.indexOf("torq-agent-") === 0) {
        return polkit.Result.NO;
      }
    });
  '';
  # The seccomp filter of `torq agent run`'s sandbox, made when the system is built (modules/agent-socket-families.c).
  socketFamilies = pkgs.runCommandCC "torq-agent-socket-families.bpf" { buildInputs = [ pkgs.libseccomp ]; } ''
    $CC -O2 -Wall -Werror -o filter ${./agent-socket-families.c} -lseccomp
    ./filter > $out
    test -s $out
  '';
  agentRun = pkgs.writeShellApplication {
    name = "torq-agent-run";
    # systemctl and sed: holding the agent's service still while a directory is shared (--hold-frozen).
    runtimeInputs = [ pkgs.bubblewrap pkgs.coreutils pkgs.util-linux pkgs.bash pkgs.jq pkgs.getent pkgs.gnused config.systemd.package
      # Whether the fence is in place for the agent, by its table (modules/model-proxy.nix).
      proxy.fenceCheck
      # torq-agent-scope, what the program is started through inside the sandbox (agent/src/scope).
      cfg.package ];
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
    # Removed with TorqOS's own agent loop (ADR-055, phase 2): `torq doctor`, `torq agent propose`, the model
    # they asked and the local model's service. Each whole subtree is one removed option, so that any setting
    # left under it in a machine's configuration is refused with these words, and not as an unknown option.
    (removed [ "torq" "agent" "doctor" ] (set: ''
      TorqOS's own agent loop is gone (`torq doctor`, `torq agent propose`): the machine's agent is the coding
      tool you choose (`set torq agent engine claude-code`).${lib.optionalString (lib.any (d: builtins.isAttrs d && (d.journal or false) == true) set) ''
       It let the agents read the system journal (`journal true`). That setting is now `torq agent journal`,
      which torqcli knows once the machine runs this TorqOS: `set torq agent journal true` is a commit of its
      own, after the update. Until then the agents do not read the journal.''}''))
    (removed [ "torq" "agent" "model" ] (_: ''
      TorqOS no longer runs a model of its own on the machine (llama.cpp, for `torq doctor`): the machine's
      agent is the coding tool you choose (`set torq agent engine claude-code`), which asks its own service.
      A model file it named by `path` is not removed from the disk: delete it yourself if you do not want it.''))
  ];

  options.torq.agent.package = lib.mkOption {
    type = lib.types.package;
    description = "torq-agent package: the agent's service and torq-mcp (set by the TorqOS flake).";
  };

  options.torq.agent.journal = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = ''
      Whether the agents may read the system journal. Off by default. On, every agent account (each
      person's, `torq-agent-<person>`) joins `systemd-journal`: every program run as the agent gains it,
      the engine and any tool a person starts with `torq agent run` (Claude Code, which sends what it reads
      to its own remote service), and TorqOS's own tool for it (torq-mcp's `read_journal`) is offered. What
      it gains is read access to every service's and every account's log; the group writes, rotates and
      deletes nothing. Off, the journal tool is not offered.
    '';
  };

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
      The people who each get an agent of their own: an account `torq-agent-<name>` (ADR-051 3) with no
      password, no login, no group but its own, and a home of its own (/var/lib/torq-agent-<name>). The
      coding tool chosen as the engine (`torq agent engine`) runs as it: for the desktop's AI and `torq ask`,
      for `claude` typed in a terminal, and for `torq agent run`. Each person's agent proposes packages only
      for that person (`users <name> packages`), and settings only as `mayPropose.settings` allows. The
      owner, by default. Each must be a person of this machine (a normal user), with a name of at most 20
      characters: NixOS takes an account's and a group's name of at most 31.
    '';
  };

  options.torq.agent.tools = lib.mkOption {
    type = lib.types.listOf (lib.types.enum [ "claude-code" ]);
    default = [ ];
    example = [ "claude-code" ];
    description = ''
      The coding tools installed for each person's agent account (ADR-051 5): `claude-code`, Claude Code. The
      program itself is in the agent accounts' own profiles, and it runs as the person's agent, never as the
      person. What people get on their PATH is a command `claude` (`torq agent plainCommand`, on by default):
      typed in a terminal, it runs Claude Code as their agent with the folder they are in shared with it;
      `torq agent run --share <folder> -- claude` is the same, spelled out. Its sign-in (a subscription's too)
      is kept in the agent's home. At each start TorqOS's tools (torq-mcp) and skill are linked into its
      settings there; what it proposes always waits for the person (ADR-051 7). Claude Code is unfree
      (its licence is not an open-source one, and nothing of that kind is installed unasked): allow it first
      (`set torq unfree packages claude-code`).
    '';
  };

  options.torq.agent.engine = lib.mkOption {
    type = lib.types.nullOr (lib.types.enum [ "claude-code" ]);
    default = null;
    example = "claude-code";
    description = ''
      The engine: the program that answers when you ask your agent. `claude-code` is Claude Code. Every place
      in TorqOS that offers AI (the agent's window and panel applet, `torq ask` at a terminal) then reaches
      it. It runs as each person's own agent account, in the same sandbox as `torq agent run`: it sees its
      own home, not yours, and can only propose a change to the system, which you approve. Install it for the
      agents first (`set torq agent tools claude-code`), and sign in once, in a terminal: `claude`, then type
      /login (`torq agent run -- claude` where `torq agent plainCommand` is off). Starting a program as the
      agent is an administrator's command today: someone who is not one asks an administrator. Not set, the
      default: the machine has no agent to ask.
    '';
  };

  options.torq.agent.plainCommand = lib.mkOption {
    type = lib.types.bool;
    default = true;
    description = ''
      Typing `claude` in a terminal runs Claude Code as your agent (`torq agent claude`, which the command
      is): the folder you are in is shared with it, as `torq agent run --share <that folder> -- claude` does,
      and whatever you type after `claude` is passed on to it unchanged. In your home folder itself, outside
      your home, or in a hidden folder of your home (~/.ssh, ~/.config: keys and settings live there) nothing
      is shared: it says so in one line and works in the agent's own folder. Your instructions for your agent
      (`torq agent instructions`) are given to it. Off: `claude` is not a command; use `torq agent run`. Only
      where Claude Code is one of `torq agent tools`. A Claude Code you installed in your own account is
      found first, and runs as you, outside the agent's sandbox.
    '';
  };

  options.torq.agent.claudeCode = lib.mkOption {
    type = lib.types.package;
    internal = true;
    default = claudeCode;
    defaultText = lib.literalExpression "pkgs.claude-code, the version modules/claude-code-manifest.json pins";
    description = "Claude Code, as `torq agent tools` installs it for the agents (a VM test puts a stand-in here: the real one is unfree, and large).";
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
        message = "torq.agent.people: ${agentOf p} (the account made for ${p}'s agent) is longer than an account's and its group's name may be (31 characters on NixOS): ${p} is too long a name for an agent of its own (at most ${toString nameLimit} characters).";
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
      # The system journal, where `torq agent journal` gives it: read access, for everything run as the agent.
      extraGroups = lib.optional cfg.journal "systemd-journal";
      # Never a user manager of its own (below).
      linger = false;
    }) agents;
    users.groups = lib.mapAttrs (_: _: { }) agents;

    # A person (an administrator) may start a program as their own agent, only through the launcher:
    # it binds the one shared directory as root, then drops to the caller's agent for good before
    # running anything (modules/agent-run.sh). No password: administrators have root already, and the
    # program runs with less than they have.
    environment.systemPackages = [ agentRun pkgs.acl ]
      ++ lib.optional (claudeChosen && claudeAllowed && cfg.plainCommand) plainClaude
      # torq-mcp, where a coding tool run as the agent finds it (/run/current-system/sw/bin/torq-mcp).
      ++ lib.optional (cfg.tools != [ ]) cfg.package;
    # What torq-mcp reads of this configuration: whether it offers its journal tool.
    environment.etc."torq/agent.json".text = builtins.toJSON { inherit (cfg) journal; };
    # The coding tools chosen (the launcher links TorqOS into them at each start; the Agent page shows them),
    # and TorqOS's skill, linked into each (ADR-051 7): advice, which the tool may rewrite.
    # And the engine among them, if one is chosen (ADR-055 1): what the person's programs say answers them.
    # And whether `claude`, typed plainly, is a command here (what a person is told to type to sign in).
    environment.etc."torq/agent-tools.json".text = builtins.toJSON { inherit (cfg) tools engine; plainCommand = claudeChosen && claudeAllowed && cfg.plainCommand; };
    environment.etc."torq/skills/torqos/SKILL.md".source = ./agent-skill.md;
    # The kinds of socket a program run as an agent may open under `torq agent run` (unix, IPv4, IPv6, netlink: what
    # the agent's service allows), as the seccomp filter bubblewrap is given (modules/agent-socket-families.c).
    environment.etc."torq/agent-socket-families.bpf".source = socketFamilies;
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

  (lib.mkIf (agents != { }) {
    # polkit decides what an account may ask of the system's services (logind: power off, reboot, lingering;
    # NetworkManager: the network), and much of it it allows a person at the machine, by the session a process is
    # in. An agent is never that person, wherever its process is: every action of polkit's is refused to every
    # agent account, before any other rule is read (the file's name sorts first). (The security reader,
    # 2026-11-17: under `torq agent run` the agent was in its person's session, and could turn on lingering for
    # itself, which gives it a user manager that runs its own units outside both sandboxes and the fence check.)
    environment.etc.${polkitRuleFile}.text = polkitRule;
    assertions = [{
      assertion = config.environment.etc.${polkitRuleFile}.enable && config.environment.etc.${polkitRuleFile}.text == polkitRule;
      message = "environment.etc.\"${polkitRuleFile}\": polkit refuses every agent account (torq-agent-<person>) everything, by TorqOS's rule and no other text: an agent is never the person at the machine.";
    }]
    # No agent account lingers: lingering is how an account gets a user manager, whose units would be its own
    # to make, outside its service's control group, its sandbox and the fence check. Off, by NixOS (its
    # linger-users unit takes away a linger file of the account at each start), whether an engine is chosen or not.
    ++ lib.mapAttrsToList (agent: _: {
      assertion = config.users.users.${agent}.linger == false;
      message = "users.users.${agent}.linger: the agent account has no user manager of its own (its units would be its own to make, outside its service's control group and its sandbox).";
    }) agents;
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
