# The check on the agent's units as built (modules/agent-units-check.nix): they are only what TorqOS's module
# writes, and nothing in them delegates. A unit file or drop-in a package brings for one of the agent's units is
# refused, whatever it sets (who the agent runs as, what it runs, its credentials, its sandbox, its socket), as
# a delegating one was before and still is, and as this configuration's own is at evaluation
# (tests/agent-assertions.nix). No VM: each case is a small directory laid out as NixOS lays out
# /etc/systemd/system (links into the store), given to the very check the module adds to the system's build.
# A case that should be refused is built with testBuildFailure, and its log must name the file.
#
# Then the same check as a real configuration carries it (its `system.checks`), with a package that brings such
# a unit file or drop-in, and without: that configuration's own unit directory, as NixOS builds it (TorqOS's
# programs replaced by stand-ins, so nothing long is built). And the flake's own hosts, the desktop on, with a
# resident agent: their unit directories pass it, so the check refuses none of what TorqOS itself puts there.
{ pkgs, self }:

let
  lib = pkgs.lib;
  check = import ../modules/agent-units-check.nix { inherit pkgs; };
  # A package's unit files, as a package has them: real files under lib/systemd/system.
  package = name: files: pkgs.runCommand name { } (''
    mkdir -p $out/lib/systemd/system
  '' + lib.concatStrings (lib.mapAttrsToList (path: text: ''
    mkdir -p "$out/lib/systemd/system/$(dirname ${lib.escapeShellArg path})"
    printf '%s' ${lib.escapeShellArg text} > $out/lib/systemd/system/${lib.escapeShellArg path}
  '') files));
  # A unit directory as NixOS makes it: a package's unit files linked in, its directories linked file by file.
  # `files` are this configuration's; `brought`, a package's. The two are told apart as NixOS's are: by where
  # each link goes. What the check is told to expect: the agent's units among `files` (unless `expected` says).
  isAgents = n: !(lib.hasInfix "/" n) && (lib.hasPrefix "torq-agent@" n || lib.hasPrefix "torq-agent-key@" n);
  laid = { name, files, brought ? { }, extra ? "", expected ? null }:
    let
      own = package "${name}-own" files;
      theirs = package "${name}-package" brought;
    in {
      units = pkgs.runCommand "${name}-units" { nativeBuildInputs = [ pkgs.lndir ]; } ''
        mkdir -p $out
        for fn in ${own}/lib/systemd/system/* ${theirs}/lib/systemd/system/*; do
          if [ ! -e "$fn" ]; then continue; fi
          if [ -d "$fn" ]; then
            mkdir -p "$out/$(basename "$fn")"
            lndir -silent "$fn" "$out/$(basename "$fn")"
          else
            ln -s "$fn" $out/
          fi
        done
        ${extra}
      '';
      expected = if expected != null then expected
        else lib.genAttrs (lib.filter isAgents (lib.attrNames files)) (n: "${own}/lib/systemd/system/${n}");
    };
  ours = "[Service]\nExecStart=/bin/true\nProtectControlGroups=true\n";
  socket = "[Socket]\nListenStream=/run/x\n";
  delegates = "[Service]\nDelegate=yes\n";
  # The agent's three units, as this configuration's.
  three = { "torq-agent@alice.service" = ours; "torq-agent@alice.socket" = socket; "torq-agent-key@alice.service" = ours; };
  passes = name: files: extra: check (laid { inherit name files extra; });
  passesWith = args: check (laid args);
  # Refused, and for that file: the failed build's log says `says`, under the heading of the rule it broke.
  delegateHeading = "no Delegate or DelegateSubgroup in any torq-agent@ unit";
  ownHeading = "the agent's units are only what TorqOS's own module writes";
  refusedWith = heading: says: args:
    pkgs.runCommand "refused-${args.name}" { failed = pkgs.testers.testBuildFailure (check (laid args)); } ''
      if ! grep -q -F -- ${lib.escapeShellArg says} "$failed/testBuildFailure.log"; then
        echo "agent-units: ${args.name}: refused, but not saying ${lib.escapeShellArg says}:" >&2
        cat "$failed/testBuildFailure.log" >&2
        exit 1
      fi
      grep -q -F ${lib.escapeShellArg heading} "$failed/testBuildFailure.log"
      touch $out
    '';
  refuses = name: files: extra: says: refusedWith delegateHeading says { inherit name files extra; };
  # Refused as not this configuration's.
  foreign = args: says: refusedWith ownHeading says args;
  # Refused for whose it is and for nothing else: no line under the Delegate rule's heading.
  foreignOnly = args: says:
    pkgs.runCommand "refused-only-${args.name}" { failed = pkgs.testers.testBuildFailure (check (laid args)); } ''
      grep -q -F -- ${lib.escapeShellArg says} "$failed/testBuildFailure.log"
      grep -q -F ${lib.escapeShellArg ownHeading} "$failed/testBuildFailure.log"
      if grep -q -F ${lib.escapeShellArg delegateHeading} "$failed/testBuildFailure.log"; then
        echo "agent-units: ${args.name}: refused under the Delegate rule too, which this case is not about" >&2
        cat "$failed/testBuildFailure.log" >&2
        exit 1
      fi
      touch $out
    '';
  cases = [
    # The controls: the agent's units as they are, a comment that names the word, another unit's own Delegate
    # (a container manager's, say), a masked unit, the directories of links.
    (passes "plain" { "torq-agent@alice.service" = ours; "torq-agent@alice.socket" = "[Socket]\nListenStream=/run/x\n"; } "")
    (passes "a-comment" { "torq-agent@alice.service" = ours + "# No Delegate here.\n; Delegate=yes\n"; } "")
    (passes "another-units" { "torq-agent@alice.service" = ours; "docker.service" = delegates; "docker.service.d/x.conf" = delegates; } "")
    (passes "masked" { "torq-agent@alice.service" = ours; } "ln -s /dev/null $out/torq-agent@bob.service")
    (passes "links-by-name" { "torq-agent@alice.service" = ours; } "mkdir $out/torq-agent@alice.service.requires && ln -s ../nothing.socket $out/torq-agent@alice.service.requires/")
    # A package's unit file or drop-in, each place systemd reads one for torq-agent@<person>.service.
    (refuses "a-packages-unit" { "torq-agent@alice.service" = delegates; } "" "torq-agent@alice.service: Delegate=yes")
    (refuses "a-packages-template" { "torq-agent@.service" = "[Service]\nDelegateSubgroup=x\n"; } "" "torq-agent@.service: DelegateSubgroup=x")
    (refuses "a-packages-drop-in" { "torq-agent@alice.service" = ours; "torq-agent@alice.service.d/10-pkg.conf" = delegates; } "" "torq-agent@alice.service.d/10-pkg.conf: Delegate=yes")
    (refuses "the-templates-drop-in" { "torq-agent@alice.service" = ours; "torq-agent@.service.d/pkg.conf" = delegates; } "" "torq-agent@.service.d/pkg.conf: Delegate=yes")
    (refuses "the-dash-drop-in" { "torq-agent@alice.service" = ours; "torq-.service.d/pkg.conf" = delegates; } "" "torq-.service.d/pkg.conf: Delegate=yes")
    (refuses "every-services-drop-in" { "torq-agent@alice.service" = ours; "service.d/pkg.conf" = delegates; } "" "service.d/pkg.conf: Delegate=yes")
    # However the line is written: spaces before the key and around the sign, "no" as well (as at evaluation).
    (refuses "spaces" { "torq-agent@alice.service" = "[Service]\n  \tDelegate = cpu memory\n"; } "" "torq-agent@alice.service: ")
    (refuses "delegate-no" { "torq-agent@alice.service" = "[Service]\nDelegate=no\n"; } "" "torq-agent@alice.service: Delegate=no")
    # NixOS's own layout when a package brings the unit this configuration also writes: the package's file
    # stays the unit, the configuration's becomes its overrides.conf. The package's Delegate is still found.
    (refuses "ours-over-a-packages" { "torq-agent@alice.service" = delegates; }
      "mkdir $out/torq-agent@alice.service.d && ln -s ${pkgs.writeText "overrides.conf" ours} $out/torq-agent@alice.service.d/overrides.conf"
      "torq-agent@alice.service: Delegate=yes")
    # A file that is not in the store cannot be read here, so is refused rather than let by.
    (refuses "a-link-out-of-the-store" { "torq-agent@alice.service" = ours; } "mkdir $out/torq-agent@alice.service.d && ln -s /etc/torq/elsewhere.conf $out/torq-agent@alice.service.d/x.conf" "x.conf: a link out of the store")
    (refuses "a-link-to-nothing" { "torq-agent@bob.service" = ours; } "ln -s /nix/store/nothing-here $out/torq-agent@alice.service" "torq-agent@alice.service: a link out of the store, or to nothing")
  ];

  # Whose the files are (2026-11-14). What a package's drop-in sets is not read: each kind of setting that would
  # change the agent is refused all the same, and for being there, not under the Delegate rule.
  settings = {
    user = "[Service]\nUser=root\n";
    group = "[Service]\nGroup=wheel\n";
    dynamic-user = "[Service]\nDynamicUser=yes\n";
    supplementary-groups = "[Service]\nSupplementaryGroups=wheel\n";
    ambient-capabilities = "[Service]\nAmbientCapabilities=CAP_SYS_ADMIN\n";
    capability-bounding-set = "[Service]\nCapabilityBoundingSet=~\n";
    no-new-privileges = "[Service]\nNoNewPrivileges=no\n";
    exec-start = "[Service]\nExecStart=\nExecStart=/bin/sh -c true\n";
    exec-start-pre = "[Service]\nExecStartPre=+/bin/sh -c true\n";
    exec-start-post = "[Service]\nExecStartPost=+/bin/sh -c true\n";
    load-credential = "[Service]\nLoadCredential=model-key:/etc/shadow\n";
    set-credential = "[Service]\nSetCredential=model-key:x\n";
    environment = "[Service]\nEnvironment=LD_PRELOAD=/tmp/x.so\n";
    environment-file = "[Service]\nEnvironmentFile=/tmp/env\n";
    pam-name = "[Service]\nPAMName=login\n";
    slice = "[Service]\nSlice=user.slice\n";
    protect-system = "[Service]\nProtectSystem=no\n";
    protect-home = "[Service]\nProtectHome=no\n";
    restrict-namespaces = "[Service]\nRestrictNamespaces=no\n";
    read-write-paths = "[Service]\nReadWritePaths=/\n";
    bind-paths = "[Service]\nBindPaths=/home\n";
    inaccessible-paths = "[Service]\nInaccessiblePaths=\n";
    system-call-filter = "[Service]\nSystemCallFilter=\n";
    working-directory = "[Service]\nWorkingDirectory=/home/alice\n";
    a-dependency = "[Unit]\nOnFailure=\nRequires=other.service\n";
  };
  ownCases =
    # Each kind of setting, in the agent's own drop-in directory.
    lib.mapAttrsToList (kind: text:
      foreignOnly { name = "sets-${kind}"; files = three; brought = { "torq-agent@alice.service.d/10-pkg.conf" = text; }; }
        "torq-agent@alice.service.d: a drop-in directory of the agent's units") settings
    ++ [
      # The controls: the three units as this configuration's; other units' drop-ins and other "torq-" units;
      # a mask of a name that is not expected, and this configuration's own mask of one that is (the expected
      # file itself a link to /dev/null); the directories of links.
      (passesWith { name = "the-three"; files = three; })
      (passesWith { name = "others-drop-ins"; files = three; brought = {
        "torq-broker.service" = ours; "torq-broker.service.d/x.conf" = settings.user; "user@.service.d/x.conf" = settings.user;
        "torq-agentx.service" = ours; "docker.socket.d/x.conf" = socket; "sockets.target.d/x.conf" = "[Unit]\nDescription=x\n";
      }; })
      (passesWith { name = "masks"; files = three; extra = "ln -s /dev/null $out/torq-agent-key@bob.service; ln -s /dev/null $out/torq-agent@bob.socket"; })
      (let masked = pkgs.runCommand "unit-torq-agent-key@alice.service" { } "mkdir $out && ln -s /dev/null $out/torq-agent-key@alice.service"; in
        passesWith {
          name = "our-own-mask";
          files = { "torq-agent@alice.service" = ours; };
          extra = "ln -s ${masked}/torq-agent-key@alice.service $out/";
          expected = {
            "torq-agent@alice.service" = "${package "our-own-mask-own" { "torq-agent@alice.service" = ours; }}/lib/systemd/system/torq-agent@alice.service";
            "torq-agent-key@alice.service" = "${masked}/torq-agent-key@alice.service";
          };
        })
      (passesWith { name = "wants"; files = three; extra = "mkdir $out/torq-agent@alice.socket.wants $out/torq-agent-key@alice.service.upholds && ln -s ../x.service $out/torq-agent@alice.socket.wants/"; })

      # Each place systemd reads a drop-in for one of the agent's units, whatever is in it.
      (foreignOnly { name = "the-templates"; files = three; brought = { "torq-agent@.service.d/pkg.conf" = settings.user; }; } "torq-agent@.service.d: a drop-in directory of the agent's units")
      (foreignOnly { name = "the-dash"; files = three; brought = { "torq-.service.d/pkg.conf" = settings.environment; }; } "torq-.service.d: a drop-in directory systemd also applies")
      (foreignOnly { name = "the-second-dash"; files = three; brought = { "torq-agent-.service.d/pkg.conf" = settings.user; }; } "torq-agent-.service.d: a drop-in directory systemd also applies")
      (foreignOnly { name = "every-service"; files = three; brought = { "service.d/pkg.conf" = settings.environment; }; } "service.d: a drop-in directory systemd also applies")
      (foreignOnly { name = "the-key-units"; files = three; brought = { "torq-agent-key@alice.service.d/pkg.conf" = settings.exec-start; }; } "torq-agent-key@alice.service.d: a drop-in directory of the agent's units")
      (foreignOnly { name = "the-key-units-template"; files = three; brought = { "torq-agent-key@.service.d/pkg.conf" = settings.exec-start; }; } "torq-agent-key@.service.d: a drop-in directory of the agent's units")
      # The socket's: who may connect, and where it listens.
      (foreignOnly { name = "the-sockets"; files = three; brought = { "torq-agent@alice.socket.d/pkg.conf" = "[Socket]\nSocketUser=bob\nSocketMode=0666\n"; }; } "torq-agent@alice.socket.d: a drop-in directory of the agent's units")
      (foreignOnly { name = "the-sockets-template"; files = three; brought = { "torq-agent@.socket.d/pkg.conf" = "[Socket]\nListenStream=/tmp/agent.sock\n"; }; } "torq-agent@.socket.d: a drop-in directory of the agent's units")
      (foreignOnly { name = "the-sockets-dash"; files = three; brought = { "torq-.socket.d/pkg.conf" = "[Socket]\nSocketMode=0666\n"; }; } "torq-.socket.d: a drop-in directory systemd also applies")
      (foreignOnly { name = "every-socket"; files = three; brought = { "socket.d/pkg.conf" = "[Socket]\nSocketMode=0666\n"; }; } "socket.d: a drop-in directory systemd also applies")
      # A drop-in directory with nothing in it, and one that masks: there, so refused (none is written).
      (foreignOnly { name = "an-empty-drop-in-directory"; files = three; extra = "mkdir $out/torq-agent@alice.service.d"; } "torq-agent@alice.service.d: a drop-in directory of the agent's units")
      (foreignOnly { name = "a-drop-in-directory-that-is-a-link"; files = three; extra = "ln -s ${package "elsewhere" { "x.conf" = settings.user; }}/lib/systemd/system $out/torq-agent@alice.service.d"; } "torq-agent@alice.service.d: a drop-in directory of the agent's units")

      # The unit itself another's. A package's file of the same name stays the unit, as NixOS lays it out (this
      # configuration's then its overrides.conf): named for both, the link and the directory.
      (let mine = pkgs.writeTextDir "torq-agent@alice.service" ours; args = {
          name = "a-packages-unit-not-ours"; files = { "torq-agent@alice.socket" = socket; }; brought = { "torq-agent@alice.service" = "[Service]\nExecStart=/bin/true\nUser=root\n"; };
          extra = "mkdir $out/torq-agent@alice.service.d && ln -s ${mine}/torq-agent@alice.service $out/torq-agent@alice.service.d/overrides.conf";
          expected = { "torq-agent@alice.service" = "${mine}/torq-agent@alice.service"; };
        }; in pkgs.linkFarmFromDrvs "a-packages-unit-not-ours" [
          (foreignOnly args "torq-agent@alice.service: not the unit this configuration writes (it is /nix/store/")
          (foreignOnly args "torq-agent@alice.service.d: a drop-in directory of the agent's units")
        ])
      (foreignOnly { name = "a-packages-socket"; files = { "torq-agent@alice.service" = ours; }; brought = { "torq-agent@alice.socket" = "[Socket]\nListenStream=/tmp/agent.sock\n"; };
          expected = { "torq-agent@alice.socket" = "${pkgs.writeTextDir "torq-agent@alice.socket" socket}/torq-agent@alice.socket"; }; }
        "torq-agent@alice.socket: not the unit this configuration writes")
      # An alias (NixOS: `aliases`): the unit's name a link to another unit's.
      (foreignOnly { name = "an-alias"; files = three; brought = { "other.service" = ours; }; extra = "ln -sfn other.service $out/torq-agent@alice.service"; }
        "torq-agent@alice.service: not the unit this configuration writes (it is other.service")
      # A file of its own in the directory, not a link: not as NixOS puts this configuration's there.
      (foreignOnly { name = "a-file-not-a-link"; files = three; extra = "rm $out/torq-agent@alice.service && printf '[Service]\\nUser=root\\n' > $out/torq-agent@alice.service"; }
        "torq-agent@alice.service: not the unit this configuration writes (a file of its own here")
      # A link to the right text in another place is still not the expected file.
      (foreignOnly { name = "the-same-text-elsewhere"; files = three; extra = "ln -sfn ${pkgs.writeText "copy" ours} $out/torq-agent@alice.service"; }
        "torq-agent@alice.service: not the unit this configuration writes")
      # A mask a package puts over a unit this configuration writes is not the expected link either.
      (foreignOnly { name = "a-mask-over-ours"; files = three; extra = "ln -sfn /dev/null $out/torq-agent-key@alice.service"; }
        "torq-agent-key@alice.service: not the unit this configuration writes (it is /dev/null")
      # A unit this configuration writes that is not there.
      (foreignOnly { name = "ours-not-there"; files = three; extra = "rm $out/torq-agent@alice.socket"; } "torq-agent@alice.socket: the unit this configuration writes is not there")
      # Other units named as the agent's are: the template, another instance, the key unit where none is
      # written, a timer and a path unit that would start the agent.
      (foreignOnly { name = "a-template"; files = three; brought = { "torq-agent@.service" = ours; }; } "torq-agent@.service: named as the agent's units are")
      (foreignOnly { name = "another-instance"; files = three; brought = { "torq-agent@mallory.service" = ours; }; } "torq-agent@mallory.service: named as the agent's units are")
      (foreignOnly { name = "a-key-unit-unasked"; files = { "torq-agent@alice.service" = ours; }; brought = { "torq-agent-key@alice.service" = "[Service]\nExecStart=/bin/sh -c true\n"; }; } "torq-agent-key@alice.service: named as the agent's units are")
      (foreignOnly { name = "a-timer"; files = three; brought = { "torq-agent@alice.timer" = "[Timer]\nOnBootSec=1\n"; }; } "torq-agent@alice.timer: named as the agent's units are")
      (foreignOnly { name = "a-socket-template"; files = three; brought = { "torq-agent@.socket" = socket; }; } "torq-agent@.socket: named as the agent's units are")
      (foreignOnly { name = "a-link-to-nothing-so-named"; files = three; extra = "ln -s /nix/store/nothing-here $out/torq-agent-key@bob.service"; } "torq-agent-key@bob.service: named as the agent's units are")
    ];

  # The check as a configuration carries it: a real configuration's unit directory, as NixOS builds it.
  # TorqOS's own programs are not what is checked here, and building them is the long part: stand-ins.
  stub = pkgs.runCommand "torq-stand-in" { version = "0"; meta.mainProgram = "torq-stand-in"; } "mkdir -p $out/bin";
  stubs = {
    torq.agent.doctor.package = stub;
    torq.broker.package = stub;
    torq.secrets.package = stub;
    torq.cli.package = stub;
  };
  configOf = extra: (import (pkgs.path + "/nixos/lib/eval-config.nix") {
    system = pkgs.stdenv.hostPlatform.system;
    modules = [ self.nixosModules.default {
      torq.user = "dev";
      boot.loader.grub.enable = false;
      fileSystems."/" = { device = "/dev/disk/by-label/root"; fsType = "ext4"; };
      torq.agent.doctor = { enable = true; endpoint = "https://api.example.org/v1"; model = "m"; };
    } stubs extra ];
  }).config;
  carried = config: lib.filter (c: (c.name or "") == "torq-agent-units-its-own") config.system.checks;
  only = found: if lib.length found == 1 then lib.head found else throw "agent-units: the configuration carries ${toString (lib.length found)} such checks, not one";
  ofSystem = extra: only (carried (configOf extra));
  withKey = { torq.agent.doctor.apiKeyFile = "/var/lib/torq/secrets/model.key"; };
  refusesSystem = name: files: says:
    pkgs.runCommand "refused-${name}" { failed = pkgs.testers.testBuildFailure (ofSystem { systemd.packages = [ (package "brings-${name}" files) ]; }); } ''
      grep -F -- ${lib.escapeShellArg says} "$failed/testBuildFailure.log"
      touch $out
    '';
  # The same, of a configuration's own doing (`extra`), and for whose the file is.
  refusesConfig = name: extra: says:
    pkgs.runCommand "refused-${name}" { failed = pkgs.testers.testBuildFailure (ofSystem { imports = [ withKey extra ]; }); } ''
      grep -F -- ${lib.escapeShellArg says} "$failed/testBuildFailure.log"
      grep -q -F ${lib.escapeShellArg ownHeading} "$failed/testBuildFailure.log"
      touch $out
    '';
  brings = name: files: { systemd.packages = [ (package "brings-${name}" files) ]; };
  # The system's build depends on the check (NixOS's system.checks: named in the system's own derivation), so a
  # configuration that fails it is not built. Read from the derivation, which evaluation gives; nothing is built.
  dependsOnIt =
    let
      config = configOf { };
      named = builtins.unsafeDiscardStringContext (toString (config.system.build.toplevel.passedChecks or ""));
      it = builtins.unsafeDiscardStringContext "${ofSystem { }}";
    in
      if lib.hasInfix it named then "the system's build depends on it"
      else throw "agent-units: the system's build does not depend on the check (${it} is not among its checks)";
  # Only where the agent is resident, as the assertions are: with doctor mode off there is no such unit.
  notWithout =
    if carried (configOf { torq.agent.doctor.enable = lib.mkForce false; }) == [ ] then "not carried where no agent is resident"
    else throw "agent-units: the check is carried where no agent is resident";
  # What the check is told to expect is what the module writes: the three units of each person where a key is
  # named, two where none is. Read at evaluation; were one dropped from the list, the check would call it another's.
  expectsAll =
    let
      names = extra: lib.sort lib.lessThan (map (l: lib.head (lib.splitString " " l)) (lib.splitString "\n" (ofSystem extra).expected));
      two = people: lib.sort lib.lessThan (lib.concatMap (p: [ "torq-agent@${p}.service" "torq-agent@${p}.socket" ]) people);
      three' = people: lib.sort lib.lessThan (two people ++ map (p: "torq-agent-key@${p}.service") people);
      both = { imports = [ withKey ]; users.users.pat.isNormalUser = true; torq.agent.people = [ "dev" "pat" ]; };
    in
      if names { } == two [ "dev" ] && names withKey == three' [ "dev" ] && names both == three' [ "dev" "pat" ] then "the check expects each unit the module writes"
      else throw "agent-units: the check expects ${builtins.toJSON [ (names { }) (names withKey) (names both) ]}";
  systemCases = [
    # The controls: the configuration as it is, and a package that brings a unit of its own, delegating (its own business).
    (ofSystem { })
    (ofSystem { systemd.packages = [ (package "brings-its-own" { "container-manager.service" = "[Service]\nExecStart=/bin/true\nDelegate=yes\n"; }) ]; })
    (refusesSystem "a-drop-in" { "torq-agent@dev.service.d/10-pkg.conf" = delegates; } "torq-agent@dev.service.d/10-pkg.conf: Delegate=yes")
    (refusesSystem "every-services-drop-in" { "service.d/10-pkg.conf" = delegates; } "service.d/10-pkg.conf: Delegate=yes")
    # The unit itself: NixOS keeps the package's file as the unit and makes this configuration's its overrides.conf.
    (refusesSystem "the-unit-itself" { "torq-agent@dev.service" = delegates; } "torq-agent@dev.service: Delegate=yes")

    # Whose the files are. The controls: a key named (the third unit), two people, a package's drop-ins for
    # other units and another "torq-" unit, and this configuration's own mask of a unit it writes.
    (ofSystem withKey)
    (ofSystem { imports = [ withKey ]; users.users.pat.isNormalUser = true; torq.agent.people = [ "dev" "pat" ]; })
    (ofSystem { imports = [ withKey (brings "others" { "torq-broker.service.d/10-pkg.conf" = settings.environment; "user@.service.d/10-pkg.conf" = settings.environment; "torq-agentx.service" = ours; }) ]; })
    (ofSystem { imports = [ withKey ]; systemd.services."torq-agent-key@dev".enable = false; })
    # A package's files, through NixOS's own laying out, none of them delegating.
    (refusesConfig "a-users-drop-in" (brings "a-users-drop-in" { "torq-agent@dev.service.d/10-pkg.conf" = settings.user; }) "torq-agent@dev.service.d: a drop-in directory of the agent's units")
    (refusesConfig "every-services" (brings "every-services" { "service.d/10-pkg.conf" = settings.environment; }) "service.d: a drop-in directory systemd also applies")
    (refusesConfig "the-dash" (brings "the-dash" { "torq-.service.d/10-pkg.conf" = settings.load-credential; }) "torq-.service.d: a drop-in directory systemd also applies")
    (refusesConfig "a-sockets-drop-in" (brings "a-sockets-drop-in" { "torq-agent@dev.socket.d/10-pkg.conf" = "[Socket]\nSocketMode=0666\n"; }) "torq-agent@dev.socket.d: a drop-in directory of the agent's units")
    (refusesConfig "the-key-units-drop-in" (brings "the-key-units-drop-in" { "torq-agent-key@dev.service.d/10-pkg.conf" = settings.exec-start; }) "torq-agent-key@dev.service.d: a drop-in directory of the agent's units")
    (refusesConfig "a-template" (brings "a-template" { "torq-agent@.service" = ours; }) "torq-agent@.service: named as the agent's units are")
    # The unit itself, a package's: it stays the unit, and this configuration's is made its overrides.conf.
    (refusesConfig "the-unit-a-packages" (brings "the-unit-a-packages" { "torq-agent@dev.service" = "[Service]\nExecStart=/bin/true\nUser=root\n"; }) "torq-agent@dev.service: not the unit this configuration writes (it is /nix/store/")
    (refusesConfig "the-unit-a-packages-overridden" (brings "the-unit-a-packages-overridden" { "torq-agent@dev.service" = "[Service]\nExecStart=/bin/true\nUser=root\n"; }) "torq-agent@dev.service.d: a drop-in directory of the agent's units; this configuration writes none (holding: overrides.conf")
    (refusesConfig "the-socket-a-packages" (brings "the-socket-a-packages" { "torq-agent@dev.socket" = "[Socket]\nListenStream=/tmp/agent.sock\n"; }) "torq-agent@dev.socket: not the unit this configuration writes")
    # This configuration's own doing, where it leaves the directory other than the module wrote it: its unit
    # asked for as a drop-in (then no unit file is there), the template's drop-in, an alias over the unit.
    (refusesConfig "ours-as-a-drop-in" { systemd.services."torq-agent@dev".overrideStrategy = "asDropin"; } "torq-agent@dev.service: the unit this configuration writes is not there")
    (refusesConfig "a-templates-drop-in-of-ours" { systemd.services."torq-agent@" = { overrideStrategy = "asDropin"; serviceConfig.User = "root"; }; } "torq-agent@.service.d: a drop-in directory of the agent's units")
    (refusesConfig "an-alias-of-ours" { systemd.services.other = { aliases = [ "torq-agent@dev.service" ]; serviceConfig.ExecStart = "/bin/true"; }; } "torq-agent@dev.service: not the unit this configuration writes (it is other.service")
  ];

  # The flake's own hosts, each with a resident agent (a key named) and the desktop on: what TorqOS itself puts
  # in the unit directory (its own units, COSMIC's packages') is not refused. Their programs are the real ones'
  # stand-ins where TorqOS's own would have to be compiled; everything else is the host's.
  hostCases =
    let
      system = pkgs.stdenv.hostPlatform.system;
      resident = { lib, ... }: {
        imports = [ stubs withKey ];
        # (The image and the installer have no owner yet: one, so that there is an agent to check.)
        torq.user = lib.mkDefault "owner";
        torq.agent.doctor = { enable = true; endpoint = "https://api.example.org/v1"; model = "m"; };
      };
      devVm = if system == "aarch64-linux" then "dev-vm-aarch64" else "dev-vm-x86_64";
      # (Not a check of nothing: the host has an agent, whose three units the check is told to expect.)
      host = name: modules:
        let c = only (carried (self.nixosConfigurations.${name}.extendModules { modules = [ resident ] ++ modules; }).config); in
        if lib.length (lib.splitString "\n" c.expected) == 3 then c else throw "agent-units: ${name}: the check expects ${c.expected}, not an agent's three units";
    in [
      (host devVm [ ])
      (host devVm [ { torq.desktop = { enable = true; settingsPackage = stub; }; } ])
      (host "image-${system}" [ ])
      (host "image-${system}" [ { torq.desktop = { enable = true; settingsPackage = stub; }; } ])
      (host "installer-${system}" [ ])
    ];
in
pkgs.runCommand "torq-agent-units" { } ''
  echo ${lib.escapeShellArg (toString (cases ++ ownCases ++ systemCases ++ hostCases ++ [ dependsOnIt notWithout expectsAll ]))} > $out
''
