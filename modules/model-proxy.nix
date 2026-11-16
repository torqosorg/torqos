# The model proxy and the agents' closed network (ADR-057).
#
# Two things, on wherever a person has an agent account (`torq.agent.people`):
#
# 1. `torq-model-proxy`, a service with an account of its own, listening on this machine only
#    (127.0.0.1, port 7801). An agent's engine asks its model there (Claude Code's ANTHROPIC_BASE_URL), and the
#    proxy passes each request to the model's service as it came and writes a line to that person's record
#    (/var/lib/torq-model-proxy/<person>/, which the person reads and no agent does). It names the caller from
#    the kernel's table of connections, never from what the caller says. It is also the agents' way to the
#    sites they may reach (HTTPS_PROXY): sign-in, the owner's list (`torq.agent.network.hosts`), every site
#    where the owner opens the web (`torq.agent.network.web`). Each site asked for is recorded, allowed or not.
#
# 2. The fence around the agents' accounts: a rule of the kernel's own packet filter, by the account a
#    connection belongs to. An agent's account reaches the proxy (and the secrets daemon, which names its
#    caller the same way and asks the person), and nothing else: no other address, of this machine or any
#    other, IPv4 or IPv6, and no DNS. Other accounts are not touched by it. The services that would look a
#    name up for anyone who asks (nscd, systemd-resolved) are closed to the agents where they run
#    (modules/agent.nix, modules/agent-run.sh) and on the system bus (below).
#
# All of `torq.*` is never an agent's to propose (the broker's rule, broker/src/agent.rs), so no agent can
# list a site or open the web. `torq.modelProxy.*` is not the configuration tree's either
# (modules/settings-fence.nix): where the proxy listens and what it is built from are TorqOS's.
{ config, lib, pkgs, ... }:

let
  cfg = config.torq.modelProxy;
  net = config.torq.agent.network;
  # Each agent account and its person (modules/agent.nix).
  agents = config.torq.agent.agents;
  enabled = agents != { };
  account = "torq-model-proxy";
  noShell = "${pkgs.shadow}/bin/nologin";
  # The record (wire/src/activity.rs, `DIR`): torqcli reads it there, as the person.
  record = "/var/lib/torq-model-proxy";
  address = "http://127.0.0.1:${toString cfg.port}";
  # What the proxy is told (proxy/src/lib.rs, `Settings`). Named in the unit by its store path, so a change
  # of any of it restarts the proxy.
  settings = pkgs.writeText "torq-model-proxy.json" (builtins.toJSON {
    inherit (cfg) port;
    inherit agents;
    inherit (net) hosts web;
  });
  secretsPort = 7800; # wire/src/secrets.rs, `PORT`
  nft = lib.getExe pkgs.nftables;
  quoted = names: lib.concatMapStringsSep ", " (n: "\"${n}\"") names;
  # The fence, as the kernel's packet filter takes it. A table of its own, replaced whole in one step (the
  # first two lines make "delete" safe where there is none yet). Only packets that belong to a socket of an
  # agent's account are judged; every other packet leaves the chain untouched (policy accept). An agent's
  # packet is let through to the proxy's port on 127.0.0.1 (and the secrets daemon's, where there is one)
  # and refused otherwise, at once, so a program fails quickly and says "connection refused".
  rules = pkgs.writeText "torq-agents.nft" ''
    table inet torq_agents
    delete table inet torq_agents
    table inet torq_agents {
      chain out {
        type filter hook output priority filter; policy accept;
        meta skuid { ${quoted (lib.attrNames agents)} } jump agent
      }
      chain agent {
        oifname "lo" ip daddr 127.0.0.1 tcp dport { ${lib.concatMapStringsSep ", " toString ([ cfg.port ] ++ lib.optional config.torq.secrets.enable secretsPort)} } accept
        meta l4proto tcp reject with tcp reset
        reject with icmpx admin-prohibited
      }
    }
  '';
in
{
  options.torq.modelProxy = {
    package = lib.mkOption {
      type = lib.types.package;
      description = "torq-model-proxy package (set by the TorqOS flake).";
    };
    port = lib.mkOption {
      type = lib.types.port;
      internal = true;
      readOnly = true;
      default = 7801;
      description = "Where the model proxy listens, on 127.0.0.1.";
    };
    agentEnvironment = lib.mkOption {
      type = lib.types.attrsOf (lib.types.strMatching "[^ \n'\"=]+");
      internal = true;
      readOnly = true;
      # Both spellings of the proxy's settings: programs differ in which they read.
      default = {
        ANTHROPIC_BASE_URL = address;
        HTTPS_PROXY = address;
        https_proxy = address;
        NO_PROXY = "127.0.0.1,localhost";
        no_proxy = "127.0.0.1,localhost";
      };
      description = "What every program run as an agent is told of the proxy: the agent's service and the launcher of `torq agent run` set it (modules/agent.nix).";
    };
    fence = lib.mkOption {
      type = lib.types.str;
      internal = true;
      readOnly = true;
      default = "torq-agent-network.service";
      description = "The unit that closes the agents' accounts' network, there wherever a person has an agent: nothing is run as an agent unless it is up (modules/agent.nix, modules/agent-run.sh).";
    };
  };

  options.torq.agent.network = {
    hosts = lib.mkOption {
      type = lib.types.listOf (lib.types.strMatching "([*][.])?[A-Za-z0-9_-]+([.][A-Za-z0-9_-]+)*");
      default = [ ];
      example = [ "github.com" "*.githubusercontent.com" ];
      description = ''
        The sites the agents may reach, besides their model and signing in to it. A name (`github.com`), or
        `*.name` for every site under it. None, by default. An agent's account reaches nothing on the network
        directly; a program run as an agent reaches a site only through TorqOS's model proxy, as HTTPS, and
        only a site listed here. Each site an agent asks for is recorded, allowed or refused (the agent's
        activity); what passes to an allowed site is encrypted between the agent and the site and is not
        recorded. A site listed here can be read by the agents, and anything they can read can be sent to it.

        Two names matter for Claude Code. Its claude.ai connectors (tools that run at Anthropic and act in
        your accounts, such as Gmail) need `api.anthropic.com` and `mcp-proxy.anthropic.com`: unlisted, the
        agents have no connectors. Listing `api.anthropic.com` also lets a program run as an agent ask the
        model directly, past the record.
      '';
    };
    web = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Whether the whole web is open to the agents. Off by default. On, a program run as an agent can read
        any website, and can send anything it can read (its own files, the folders you share with it, what
        its tools return) to any website. The record then shows only which sites it reached and when, not
        what was sent or read. Off, the agents reach only the sites in `torq agent network hosts`. Claude
        Code's own web search still works when this is off (it runs at Anthropic); its fetching of a page
        you name does not.
      '';
    };
  };

  config = lib.mkIf enabled {
    assertions = [
      {
        # The proxy's account is what it is said to be, whatever else the configuration says of it, as the
        # checker's is (modules/broker.nix): each line is one way it could become more (tests/agent-assertions.nix).
        assertion = let u = config.users.users.${account}; in
          u.isSystemUser && !u.isNormalUser && u.group == account && u.extraGroups == [ ]
          && lib.all (g: !(builtins.elem account g.members)) (lib.attrValues config.users.groups)
          # No one else is in its group: the record's files are readable by that group.
          && config.users.groups.${account}.members == [ ]
          && lib.all (n: n == account || config.users.users.${n}.group != account) (lib.attrNames config.users.users)
          && u.hashedPassword == "!" && u.password == null && u.hashedPasswordFile == null
          && u.initialPassword == null && u.initialHashedPassword == null
          && u.shell == noShell && u.home == "/var/empty" && !u.createHome
          && u.openssh.authorizedKeys.keys == [ ] && u.openssh.authorizedKeys.keyFiles == [ ];
        message = "users.users.${account}: the model proxy's account (ADR-057) is a system account in its own group and no other, alone in that group, with no password, no shell, no home and no SSH key: it sees what every agent asks its model, and writes each person's record.";
      }
      {
        # The fence is a table of its own in the kernel's packet filter. NixOS's nftables service, told to
        # empty everything before it loads its own rules, would empty the fence too.
        assertion = !(config.networking.nftables.enable && config.networking.nftables.flushRuleset);
        message = "networking.nftables.flushRuleset: with it, NixOS's firewall empties every table of the packet filter when it loads, and the fence that closes the agents' accounts' network (table inet torq_agents, ADR-057) with them. Leave it off (NixOS then replaces only its own tables), or take the agents away (`delete torq agent people`).";
      }
    ];

    users.users.${account} = {
      isSystemUser = true;
      group = account;
      description = "TorqOS model proxy (passes on what the agents ask their model, and records it)";
      hashedPassword = "!";
      shell = noShell;
      home = "/var/empty";
      createHome = false;
    };
    users.groups.${account} = { };

    # The record: a folder for each person, the proxy's own. The person reads it by an access rule for them
    # alone (their own group is everyone's, `users`); files made in it inherit the rule. No one else but root:
    # not another person, not any agent's account. The top folder can be passed through, not listed.
    systemd.tmpfiles.settings."10-torq-model-proxy" = {
      ${record}.d = { mode = "0711"; user = account; group = account; };
    } // lib.mapAttrs' (_: person: lib.nameValuePair "${record}/${person}" {
      d = { mode = "0700"; user = account; group = account; };
      # The person, here and in every file made here; the rule for new files said whole, so that the folder's
      # group (which no one is in) is given nothing by it.
      "a+".argument = "u:${person}:r-x,d:u::rwx,d:g::---,d:o::---,d:u:${person}:r-x";
    }) agents;

    systemd.services.torq-model-proxy = {
      description = "TorqOS model proxy: what the agents ask their model, recorded; the sites they may reach (ADR-057)";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" "systemd-tmpfiles-setup.service" ];
      serviceConfig = {
        ExecStart = "${lib.getExe cfg.package} serve ${settings}";
        User = account;
        Group = account;
        Restart = "on-failure";
        RestartSec = 2;
        # It writes the record and nothing else; a file of it is the proxy's and readable by its group, which
        # is what a person's access rule is limited by.
        ReadWritePaths = [ record ];
        UMask = "0027";
        # What passes through it is what the agents read: no core of it.
        LimitCORE = 0;
        NoNewPrivileges = true;
        CapabilityBoundingSet = "";
        AmbientCapabilities = "";
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectKernelLogs = true;
        ProtectControlGroups = true;
        ProtectClock = true;
        ProtectHostname = true;
        # Not a /proc of its own kind (ProcSubset) and not a user namespace (PrivateUsers): it reads the
        # kernel's table of connections there (/proc/net/tcp), with the accounts' real numbers.
        ProtectProc = "invisible";
        # The network, and unix sockets for the machine's own name lookups.
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_UNIX" ];
        RestrictNamespaces = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = [ "@system-service" "~@privileged" ];
        # Bounded: so many requests at once for each agent, each read up to a bound (wire/src/activity.rs).
        MemoryMax = "2G";
        TasksMax = 128;
        LimitNOFILE = 8192;
      };
    };

    # The fence. Loaded before the network is up and before anything can run as an agent; replaced in one
    # step when the agents change (reload, not restart: a restart would stop the agents' services, which
    # require it). Stopped, the table goes, and so does every agent's service; `torq agent run` refuses to
    # start anything while it is not up.
    systemd.services.torq-agent-network = {
      description = "TorqOS: the agents' accounts reach the network only through the model proxy (ADR-057)";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-pre.target" ];
      before = [ "network-pre.target" ];
      after = [ "systemd-sysusers.service" "systemd-modules-load.service" ];
      reloadIfChanged = true;
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${nft} -f ${rules}";
        ExecReload = "${nft} -f ${rules}";
        ExecStop = "${nft} delete table inet torq_agents";
      };
    };

    # systemd-resolved answers a name for anyone on the system bus. An agent's account is refused there: a
    # name looked up is a message sent (to whoever runs the name's servers), and the fence closes DNS.
    services.dbus.packages = [
      (pkgs.writeTextDir "share/dbus-1/system.d/torq-agents.conf" ''
        <?xml version="1.0"?>
        <!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
          "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
        <busconfig>
        ${lib.concatMapStrings (a: ''
          <policy user="${a}">
            <deny send_destination="org.freedesktop.resolve1"/>
          </policy>
        '') (lib.attrNames agents)}</busconfig>
      '')
    ];

    # Claude Code's settings for the agents (ADR-057): where its model is and how it reaches a site. Claude
    # Code reads /etc/claude-code/managed-settings.json before every other settings file, and the account
    # cannot override it. That file would hold for every account on the machine, so it is not put there: it is
    # kept here, and shown as /etc/claude-code only inside an agent's sandbox (the agent's service and `torq
    # agent run`, modules/agent.nix). A person's own Claude Code sees an /etc/claude-code with no settings in it.
    environment.etc."torq/claude-code/managed-settings.json".text = builtins.toJSON { env = cfg.agentEnvironment; };
    environment.etc."claude-code/README".text = ''
      TorqOS keeps no settings for Claude Code here: a Claude Code you run as yourself is not managed by it.
      Inside an agent's account this folder shows the agents' settings instead (/etc/torq/claude-code): the
      agents ask their model through TorqOS's model proxy, which keeps a record of each request.
    '';
    # What the launcher of `torq agent run` sets for every program it runs as an agent, one NAME=value a line.
    environment.etc."torq/agent-environment".text = lib.concatStrings (lib.mapAttrsToList (k: v: "${k}=${v}\n") cfg.agentEnvironment);
  };
}
