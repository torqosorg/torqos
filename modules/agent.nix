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
  # Each person's resident agent's own account (ADR-051 3, amended 2026-11-11): its service runs as it, never as
  # the agent account any program run as the agent has.
  residentOf = person: "torq-resident-${person}";
  fits = p: lib.stringLength (residentOf p) <= 31;
  # The launcher's sudo rules that would let its caller set the environment (SETENV), as text or as rules.
  isLauncher = c: lib.hasInfix "torq-agent-run" (if builtins.isString c then c else c.command);
  launcherSetenv = lib.filter (c: isLauncher c && !(builtins.isString c) && builtins.elem "SETENV" c.options) (lib.concatMap (r: r.commands) config.security.sudo.extraRules)
    ++ lib.filter (l: lib.hasInfix "torq-agent-run" l && lib.hasInfix "SETENV" l) (lib.splitString "\n" config.security.sudo.extraConfig);
  personsLikeAgents = lib.attrNames (lib.filterAttrs (n: u: u.isNormalUser && (lib.hasPrefix "torq-agent" n || lib.hasPrefix "torq-resident" n)) config.users.users);
  agents = lib.listToAttrs (map (p: lib.nameValuePair (agentOf p) p) (lib.filter fits cfg.people));
  # Where the resident service runs (its socket, its unit): the resident accounts, and the broker's map of them.
  resident = cfg.service.enable && config.torq.agent.doctor.enable;
  # Any torq-agent@ unit this configuration writes (an instance or the template) whose text delegates its control
  # group. A unit file a package brings (systemd.packages) is not in systemd.units, so not seen here: it is a file
  # inside a built package, which evaluation cannot read. Those are read in the units as built (unitsCheck).
  delegatedUnits = lib.attrNames (lib.filterAttrs (n: u: lib.hasPrefix "torq-agent@" n && lib.hasInfix "Delegate" (if u.text or null == null then "" else u.text)) config.systemd.units);
  unitsCheck = import ./agent-units-check.nix { inherit pkgs; };
  # The agent's units as this module writes them (below), each with the one file NixOS makes of it: what the
  # check on the units as built holds the finished directory to. Nothing else there may be named as they are,
  # and none may have a drop-in.
  ownUnits = lib.concatMap (person: [ "torq-agent@${person}.service" "torq-agent@${person}.socket" ]
    ++ lib.optional (modelKey != null) "torq-agent-key@${person}.service") (lib.attrValues agents);
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
    };
  }) agents;
  ownServices = lib.mapAttrs' (agent: person: lib.nameValuePair "torq-agent@${person}" {
    description = "${person}'s TorqOS agent (proposes; a person applies)";
    requires = [ "torq-agent@${person}.socket" ];
    after = [ "torq-agent@${person}.socket" "torq-broker.socket" ];
    # A start that failed: if the model's key is missing, TorqOS says so in this unit's journal (keyMissing).
    onFailure = lib.optional (modelKey != null) "torq-agent-key@${person}.service";
    serviceConfig = {
      ExecStart = "${config.torq.agent.doctor.package}/bin/torq-agent-service ${person}";
      # The listening socket as standard input (Accept=no): the program takes it from there.
      StandardInput = "socket";
      StandardOutput = "journal";
      StandardError = "journal";
      # Its own account, never the agent's (ADR-051 3 amended 2026-11-11).
      User = residentOf person;
      Group = residentOf person;
      # Its memory and its programs' home (`home`, which the service makes): the agent's, 0700, outside the
      # agent's home, hidden from `torq agent run` (modules/agent-run.sh). The agent's home itself is out of
      # its reach (below): every program run as the agent can write there, and nothing it leaves is read by
      # what runs in this unit — which is why the broker may take this unit for the resident agent (ADR-051
      # step 5).
      WorkingDirectory = "/var/lib/torq-agent-memory/${person}";
      StateDirectory = "torq-agent-memory/${person}";
      StateDirectoryMode = "0700";
      InaccessiblePaths = [ "-/var/lib/${agent}" ];
      # The model's key (a file only root reads): this unit's own credential, in a place only it sees; hidden
      # from `torq agent run` besides (modules/agent-run.sh hides /run/credentials). ADR-051 step 5, amended.
      LoadCredential = lib.optional (modelKey != null) "model-key:${toString modelKey}";
      # (As root, outside the sandbox, as `+` runs a command: it also takes away root's word of a missing
      # key, the person's, in /run.)
      ExecStartPre = lib.optional (modelKey != null) "+${keyCheck} ${person}";
      # The launcher's sandbox: every home (and /root, and the people's runtime directories in
      # /run/user) out of reach, /tmp its own, no privilege at all.
      ProtectHome = true;
      PrivateTmp = true;
      NoNewPrivileges = true;
      CapabilityBoundingSet = "";
      AmbientCapabilities = "";
      # And more: the system read-only but its memory; no other user's processes seen; the kernel's
      # settings, modules, logs and clock, and the control groups, untouchable (no Delegate: a process of
      # the agent's cannot move itself into this unit's group, whose cgroup.procs is root's).
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
      MemoryDenyWriteExecute = true;
      RemoveIPC = true;
      PrivateIPC = true;
      KeyringMode = "private";
      UMask = "0077";
      # Unix sockets (the broker, the local model), the network (a remote model), and netlink: the
      # doctor's `ip` and `iw` read the interfaces through it (hardware_facts).
      RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" "AF_NETLINK" ];
      SystemCallFilter = [ "@system-service" ];
      SystemCallErrorNumber = "EPERM";
      SystemCallArchitectures = "native";
      # Not PrivateUsers: in a user namespace of its own, SO_PEERCRED would name every caller nobody.
      # Not PrivateDevices: the doctor's rfkill and lsblk read devices.
    };
  }) agents
  # What says a missing key (keyMissing), started only by the agent unit's failure. Root, as the check at the
  # start is, since the key's directory is root's alone to look into; but with one capability (to look, not to
  # write), a read-only system and no new privilege. It is given no credential and never opens the key.
  // lib.optionalAttrs (modelKey != null) (lib.mapAttrs' (agent: person: lib.nameValuePair "torq-agent-key@${person}" {
    description = "${person}'s TorqOS agent: says a missing model key";
    # Never held back for starting often: it runs once for each failed start, which systemd already limits.
    unitConfig.StartLimitIntervalSec = 0;
    serviceConfig = {
      Type = "oneshot";
      ExecStart = "${keyMissing} torq-agent@${person}.service";
      CapabilityBoundingSet = "CAP_DAC_READ_SEARCH";
      NoNewPrivileges = true;
      ProtectSystem = "strict";
      # The one place it writes: its word for the person (keyMissingDir, root's own directory, where being
      # its owner is enough to write). Not there, it still says its line in the journal.
      ReadWritePaths = [ "-${keyMissingDir}" ];
    };
  }) agents);
  residents = lib.optionalAttrs resident (lib.listToAttrs (map (p: lib.nameValuePair (residentOf p) p) (lib.filter fits cfg.people)));
  # Claude Code for the agents (ADR-051 5, `torq agent tools`): the version dev mode pins (modules/dev.nix), unfree,
  # so only where `torq unfree` allows it — else the assertion below says so, and nothing unfree is evaluated.
  claudeChosen = builtins.elem "claude-code" cfg.tools;
  claudeAllowed = config.torq.unfree.all || builtins.elem "claude-code" config.torq.unfree.packages;
  claudeCode = pkgs.claude-code.override { manifest = lib.importJSON ./claude-code-manifest.json; };
  # The model key, checked where it is read, at each start of a resident agent, as root (`+`): a plain file, root's,
  # that no one else may read (step 5's second review: the configuration can only check the path's name). Otherwise
  # the unit does not start, and its journal says why.
  modelKey = config.torq.agent.doctor.apiKeyFile;
  # Root's word, for the person, that their agent could not start for want of the key (keyMissing writes it;
  # torqcli's client reads that it is there, cli/src/agent/client.rs KEY_MISSING_DIR): an empty file named for
  # the person, in a directory only root writes. It holds nothing of the key, not even its path.
  keyMissingDir = "/run/torq/agent-key-missing";
  keyCheck = pkgs.writeShellScript "torq-agent-key-check" ''
    f=${lib.escapeShellArg (toString modelKey)}
    # This command runs, so systemd has loaded the unit's credential: the key is there. Root's word that it was
    # missing is taken away before the key is judged below, since a key that is there and wrong is not a key
    # missing. ($1: the person, a login name; never fails the start.)
    if [ -n "''${1-}" ]; then ${pkgs.coreutils}/bin/rm -f -- ${keyMissingDir}/"$1" || true; fi
    if [ -L "$f" ] || [ ! -f "$f" ]; then
      echo "torq-agent: the model's key, $f, is not a plain file (a link is not followed): the resident agent is not started" >&2
      exit 1
    fi
    owner=$(${pkgs.coreutils}/bin/stat -c %u -- "$f")
    mode=$(${pkgs.coreutils}/bin/stat -c %a -- "$f")
    if [ "$owner" != 0 ] || [ $(( 0$mode & 077 )) -ne 0 ]; then
      echo "torq-agent: the model's key, $f, must be root's and readable by no one else (sudo chown root:root $f; sudo chmod 600 $f): the resident agent is not started" >&2
      exit 1
    fi
  '';
  # A key that is not there at all. systemd refuses the unit before any of its commands runs: it loads a unit's
  # credentials for every command it starts, ExecCondition and ExecStartPre included (tried on systemd 260), so
  # the check above never gets to speak. This runs after that failure, as a unit of its own that the agent's unit
  # names in OnFailure. It looks only at whether anything is at the path, never at what the file holds, and
  # writes TorqOS's own line into the journal as an entry about the agent's unit (OBJECT_SYSTEMD_UNIT, which
  # `journalctl -u` and `systemctl status` show when root wrote it). It changes nothing on the way to a start:
  # no key, no start, as before.
  keyMissing = pkgs.writeShellScript "torq-agent-key-missing" ''
    f=${lib.escapeShellArg (toString modelKey)}
    unit=$1
    # The same said to the person, who cannot read the journal or look into the key's directory: an empty file
    # of root's, named for them (the unit's instance), that torqcli's client finds after its failure. Only that
    # it is there is read. It is taken away as soon as it is no longer so: here, when the key is found there,
    # and at the agent's next start that loads its key (keyCheck).
    person=''${unit#torq-agent@}
    person=''${person%.service}
    word=${keyMissingDir}/$person
    if [ -e "$f" ]; then
      ${pkgs.coreutils}/bin/rm -f -- "$word" || true
      exit 0
    fi
    msg="torq-agent: the model's key, $f, is missing (torq.agent.doctor.apiKeyFile names it; a link to nothing counts as missing): the resident agent is not started without it. Put the key there as root's alone: sudo install -D -o root -g root -m 600 <the key's file> $f"
    printf 'MESSAGE=%s\nPRIORITY=3\nSYSLOG_IDENTIFIER=torq-agent\nOBJECT_SYSTEMD_UNIT=%s\n' "$msg" "$unit" | ${pkgs.util-linux}/bin/logger --journald || echo "$msg" >&2
    # Made anew each time (never written through whatever was there), its time this failure's.
    umask 022
    ${pkgs.coreutils}/bin/rm -f -- "$word" || true
    ${pkgs.coreutils}/bin/touch -- "$word" || echo "torq-agent: could not leave word of the missing key at $word" >&2
    # The key put there while this ran: a start that loaded it may already have taken the word away, before it
    # was written again just above. Not left to say what is no longer so.
    if [ -e "$f" ]; then ${pkgs.coreutils}/bin/rm -f -- "$word" || true; fi
  '';
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

  options.torq.agent.claudeCode = lib.mkOption {
    type = lib.types.package;
    internal = true;
    default = claudeCode;
    defaultText = lib.literalExpression "pkgs.claude-code, the version modules/claude-code-manifest.json pins";
    description = "Claude Code, as `torq agent tools` installs it for the agents (a VM test puts a stand-in here: the real one is unfree, and large).";
  };

  options.torq.agent.service.enable = lib.mkOption {
    type = lib.types.bool;
    default = true;
    description = ''
      Whether each person in `people` has their agent resident (ADR-051 3): `torq-agent@<person>.service`,
      started by its socket (`/run/torq-agent/<person>.sock`, theirs alone) the first time they `torq ask`,
      as their agent account, in a sandbox. It keeps their conversation (its last turns) and runs one task
      at a time. Only where doctor mode is on (`torq.agent.doctor.enable`): its harness and its model are
      the doctor's. Not tied to the desktop: `torq ask` works on a server's terminal too.
    '';
  };

  options.torq.agent.residents = lib.mkOption {
    type = lib.types.attrsOf lib.types.str;
    internal = true;
    readOnly = true;
    default = residents;
    description = "Each resident agent's own account and the person it serves (the broker's map, /etc/torq/broker.json; ADR-051 3 amended 2026-11-11).";
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
    environment.etc."torq/agent-tools.json".text = builtins.toJSON { inherit (cfg) tools; };
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

  # Each person's agent, resident (ADR-051 3, step 2): `torq-agent@<person>.service`, started by its
  # socket the first time the person asks (`torq ask`), so it costs nothing until then. It holds the
  # person's conversation and runs one task at a time, each TorqOS's harness (`torq-doctor --propose
  # --task`) as its own process, as the agent, in the sandbox below — at least the launcher's (every
  # home hidden, the people's runtime directories hidden, its own /tmp, no privilege, its home its own)
  # and systemd's hardening besides. Only where doctor mode is on: the harness and its model are its.
  (lib.mkIf (cfg.service.enable && config.torq.agent.doctor.enable) {
    # What the broker's resident rule rests on (ADR-051 3 amended 2026-11-11, socket::found_resident), each said.
    # The primary defence is the uid: the resident's own account, which nothing run as the agent account is. The
    # second lines: the unit's control group is root's to fill (nothing delegated, in any torq-agent@ unit this configuration writes — an
    # instance or the template); no agent or resident account has a user manager to make units with;
    # a same-uid program may trace only its own descendants.
    assertions = lib.concatLists (lib.mapAttrsToList (agent: person:
      let sc = config.systemd.services."torq-agent@${person}".serviceConfig; in [
        {
          assertion = !(sc ? Delegate) && !(sc ? DelegateSubgroup);
          message = "systemd.services.\"torq-agent@${person}\": no Delegate or DelegateSubgroup — a second line behind the resident account's uid: the broker also takes this unit's control group for ${person}'s resident agent, which only root may move a process into.";
        }
        {
          assertion = (config.users.users.${agent}.linger or null) != true && (config.users.users.${residentOf person}.linger or null) != true;
          message = "users.users.${agent}.linger, users.users.${residentOf person}.linger: neither account has a user manager of its own (a second line behind the uids: its units would be its own to make, outside the resident agent's control group).";
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
        message = "systemd.units ${lib.concatStringsSep ", " delegatedUnits}: no Delegate or DelegateSubgroup in any torq-agent@ unit, an instance or the template — a second line behind the resident account's uid (the unit's control group, root's to fill).";
      }
      {
        assertion = (config.boot.kernel.sysctl."kernel.yama.ptrace_scope" or 0) >= 1;
        message = "boot.kernel.sysctl.\"kernel.yama.ptrace_scope\": at least 1 where the agent is resident — a second line behind the uids: a program of the agent's may trace only its own descendants.";
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
    # Each person's resident agent's own account (ADR-051 3, amended 2026-11-11): hidden — no password, no login,
    # no home, no group but its own — so that what its service keeps (its memory, its programs' home, the model's
    # key as its credential) is its own, and nothing any program run as the agent account can write is read by
    # it. The broker counts it as the person's one agent (broker.json's `residents`).
    users.users = lib.mapAttrs (account: person: {
      isSystemUser = true;
      group = account;
      description = "${person}'s TorqOS agent, resident (its service's own account; proposes, a person applies)";
      home = "/var/empty";
      createHome = false;
      hashedPassword = "!";
    }) residents;
    users.groups = lib.mapAttrs (_: _: { }) residents;
    systemd.sockets = ownSockets;
    systemd.services = ownServices;
    # Root's, and only root writes there: a person reads that their word is there, and can put none of their
    # own. In /run, so no word outlives a restart of the machine.
    systemd.tmpfiles.settings."10-torq-agent" = lib.mkIf (modelKey != null) {
      ${keyMissingDir}.d = { mode = "0755"; user = "root"; group = "root"; };
    };
  })
  ];
}
