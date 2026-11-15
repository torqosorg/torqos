# The agent's units, as built: only what this module writes, word for word, and nothing delegated
# (modules/agent.nix; KNOWN-GAPS 6zzo).
#
# The configuration's own checks (agent.nix's assertions) read `systemd.units`: what this configuration writes.
# A unit file or drop-in that a package brings (`systemd.packages`: its lib/systemd/system and
# etc/systemd/system) is not there. It is a file inside a built package, and evaluation cannot read it without
# building the package first. So this reads the finished directory instead, the one NixOS makes
# /etc/systemd/system from, where the packages' files and the configuration's are already put together. It is
# a build of its own that the system's build depends on (`system.checks`): a system that fails it is not built,
# so never switched to. There is no setting that turns it off or lets one file by.
#
# `units`: that directory. `expected`: the agent's units as the module writes them, each unit's name and the one
# file in the store it must be (`systemd.units.<name>.unit`, the derivation NixOS makes from the unit's text).
# `intended`: for each of the same names, the text the module means the unit to have (agent.nix, intendedText).
#
# Three rules, each said apart in the build's log.
#
# 1. Whose the files are (an allow-list, 2026-11-14). A unit's settings come from its unit file and from every
#    drop-in systemd applies to it, and any of them may change who the agent runs as, what it runs, its
#    credentials or its sandbox (User, ExecStart, LoadCredential, Protect…, a socket's ListenStream and
#    SocketUser: there is no end to that list). So no key is looked for. Instead:
#    - each expected unit is there, and is a link to exactly its expected file. NixOS links a unit of the
#      configuration's as `<name> -> <its derivation>/<name>`, and a package's as `<name> -> <package>/…/<name>`:
#      the link's own text says whose it is. A package that brings a unit of the same name stays the unit (the
#      configuration's becomes its `overrides.conf`), and an alias (`aliases`) replaces the link with another
#      unit's name: neither link is then the expected one;
#    - nothing else is named as the agent's units are (`torq-agent@…`: another instance, the template, a timer
#      or a path unit of that name; and `torq-agent-key@…`, the name the old loop's key unit had, which nothing
#      writes any more and which stays refused, so that a unit so named is never taken for TorqOS's), but a mask
#      (a link that ends at /dev/null: it can only keep a unit from starting);
#    - no other name is a link to one of the agent's units (an alias of it: `other.service ->
#      torq-agent@<person>.service`). systemd applies an alias's own drop-ins (`other.service.d`) to the unit
#      it names, so an alias would be a way round the next line. The module gives its units no other name;
#    - none of the agent's units has a drop-in directory, and none of the other directories systemd applies to
#      them is there. The list is systemd's own search, read in the source of the systemd this flake builds
#      (260.5, src/shared/dropin.c, unit_file_find_dirs), for `torq-agent@<person>.service` and the socket
#      (and for a unit named `torq-agent-key@<person>.service`, the old loop's, refused as above):
#        the unit's own                      torq-agent@<person>.service.d
#        its template's                      torq-agent@.service.d
#        the name cut at each dash of the part before the `@`, as a template and with the instance kept
#                                            torq-.service.d, torq-@.service.d, torq-@<person>.service.d
#                                            (and for the old key unit's name torq-agent-.service.d,
#                                            torq-agent-@.service.d, torq-agent-@<person>.service.d)
#        every unit's of the type            service.d, socket.d
#      A dash in the person's name cuts nothing: only the part before the `@` is cut. The module writes no
#      drop-in, so any that is there is someone else's: NixOS's `overrides.conf` included, which is there only
#      when another file is the unit, or when the configuration asks for its unit as a drop-in. Each file found
#      is named in the log with the store path it comes from.
#    Not read, and why that is safe: `<unit>.wants`, `<unit>.requires` and `<unit>.upholds`, whether they are
#    the agent's units' own or name the agent's units inside another's. systemd takes from such a directory
#    only the names of the links in it (src/core/load-dropin.c, process_deps: the name is added as a
#    dependency, the link's target is not loaded), so they set nothing in the agent's unit: not who it runs as,
#    what it runs, its credentials or its sandbox. They can start another unit beside the agent (that unit is
#    its own, with its own settings, and whoever can add it here can add it to any target), start the agent
#    earlier than its socket would, or, with `.requires`, keep it from starting when the other fails: no more
#    than a mask does.
#
# 2. Nothing delegated (as before): a line that sets a key beginning with `Delegate` (Delegate,
#    DelegateSubgroup) in any `torq-agent@` unit file or drop-in directory, in `torq-.service.d` and in
#    `service.d`, and a file there that is not in the store (a link out of it, or to nothing: what it would say
#    on the machine is not known here; /dev/null, a masked unit, is let by). Rules 1 and 3 leave this only the
#    module's own files to find it in; it stays, a second reading of the same directory.
#
# 3. What the files say (2026-11-14). Rule 1 holds each unit to "the file this configuration wrote", and the
#    configuration can be made to write another: `systemd.services."torq-agent@<person>".serviceConfig.User =
#    lib.mkForce "root"` in any module, or `systemd.units.<name>.text` forced. So each expected unit's file must
#    also be, byte for byte, the text the module means (`intended`): the whole text, not chosen settings, for
#    the reason rule 1 gives. A difference is said line by line, with the unit's name and the setting. A unit
#    this configuration masks (the file a link to /dev/null) is let by: it does not start at all.
{ pkgs }:

{ units, expected, intended }:
pkgs.runCommand "torq-agent-units-its-own"
  {
    inherit units;
    # Each unit's name, the file it must be, and a file holding the text it must have.
    expected = pkgs.lib.concatStringsSep "\n" (pkgs.lib.mapAttrsToList
      (name: file: "${name} ${file} ${pkgs.writeText "torq-agent-unit-as-written" intended.${name}}") expected);
  } ''
  # Rule 1: whose the files are.
  foreign=$TMPDIR/foreign
  : > "$foreign"
  # Rule 3: what they say.
  changed=$TMPDIR/changed
  : > "$changed"
  generic=
  declare -A want text
  while IFS=' ' read -r name file written; do
    if [ -n "$name" ]; then want[$name]=$file; text[$name]=$written; fi
  done <<< "$expected"
  # Each file in a drop-in directory, with where it really is: the package (the store path) it comes from.
  holds() {
    local f real
    while IFS= read -r -d "" f; do
      real=$(realpath -m -- "$f" 2>/dev/null || true)
      case "$real" in
        /nix/store/*/*) echo "  ''${f#"$units"/}: from $(echo "$real" | cut -d/ -f1-4) (the file is $real)" ;;
        *) echo "  ''${f#"$units"/}: a link to ''${real:-nothing that can be read}, not in the store" ;;
      esac
    done < <(find -L "$1" ! -type d -print0 2>/dev/null | sort -z)
  }
  for entry in "$units"/*; do
    if [ ! -e "$entry" ] && [ ! -L "$entry" ]; then continue; fi
    name=''${entry##*/}
    case "$name" in
      # Only links to other units, by name: nothing is read from them as settings (see above).
      *.wants | *.requires | *.upholds) continue ;;
      service.d | socket.d | torq-.service.d | torq-.socket.d | torq-agent-.service.d | torq-agent-.socket.d \
      | torq-@*.service.d | torq-@*.socket.d | torq-agent-@*.service.d | torq-agent-@*.socket.d)
        echo "$name: a drop-in directory systemd also applies to the agent's units; this configuration writes none" >> "$foreign"
        holds "$entry" >> "$foreign"
        generic=1
        continue
        ;;
      torq-agent@* | torq-agent-key@*) ;;
      *)
        # Another name that is a link to one of the agent's units: an alias, whose drop-ins would be the agent's.
        if [ -L "$entry" ]; then
          to=$(readlink -- "$entry")
          case "''${to##*/}" in
            torq-agent@* | torq-agent-key@*)
              echo "$name: another name for the agent's unit ''${to##*/} (a link to it; systemd would apply $name.d to the agent's unit); this configuration gives the agent's units no other name" >> "$foreign"
              ;;
          esac
        fi
        continue
        ;;
    esac
    case "$name" in
      *.d)
        echo "$name: a drop-in directory of the agent's units; this configuration writes none (holding: $(ls -A -- "$entry" 2>/dev/null | tr '\n' ' '))" >> "$foreign"
        holds "$entry" >> "$foreign"
        continue
        ;;
    esac
    if [ -n "''${want[$name]+set}" ]; then
      if [ ! -L "$entry" ]; then
        echo "$name: not the unit this configuration writes (a file of its own here, not a link to ''${want[$name]})" >> "$foreign"
      elif [ "$(readlink -- "$entry")" != "''${want[$name]}" ]; then
        echo "$name: not the unit this configuration writes (it is $(readlink -- "$entry"), not ''${want[$name]})" >> "$foreign"
      else
        # The file this configuration wrote: rule 3, what it says.
        real=$(realpath -e -- "$entry" 2>/dev/null || true)
        case "$real" in
          /dev/null) ;;
          /nix/store/*)
            if [ ! -f "$real" ] || [ ! -r "$real" ]; then
              echo "$name: not a file that can be read" >> "$changed"
            elif ! cmp -s -- "''${text[$name]}" "$real"; then
              said=
              while IFS= read -r line; do
                case "$line" in
                  '< '?*) l=''${line#< }; said=1; echo "$name: ''${l%%=*}: the unit as built lacks \"$l\", which TorqOS's module writes" >> "$changed" ;;
                  '> '?*) l=''${line#> }; said=1; echo "$name: ''${l%%=*}: the unit as built says \"$l\", which TorqOS's module does not write" >> "$changed" ;;
                esac
              done < <(diff -- "''${text[$name]}" "$real" || true)
              if [ -z "$said" ]; then
                echo "$name: the unit as built is not, byte for byte, what TorqOS's module writes (its empty lines, or its last line's end)" >> "$changed"
              fi
            fi
            ;;
          *) echo "$name: a link out of the store, or to nothing: what it says on the machine is not known here" >> "$changed" ;;
        esac
      fi
    elif [ "$(realpath -e -- "$entry" 2>/dev/null || true)" != /dev/null ]; then
      echo "$name: named as the agent's units are, and not one this configuration writes" >> "$foreign"
    fi
  done
  for name in "''${!want[@]}"; do
    if [ ! -L "$units/$name" ] && [ ! -e "$units/$name" ]; then
      echo "$name: the unit this configuration writes is not there" >> "$foreign"
    fi
  done

  # Rule 2: nothing delegated.
  bad=$TMPDIR/bad
  : > "$bad"
  for entry in "$units"/torq-agent@* "$units"/torq-.service.d "$units"/torq-@*.service.d "$units"/service.d; do
    # (No such entry: the pattern itself, or a name that is not there. A link to nothing is looked at below.)
    if [ ! -e "$entry" ] && [ ! -L "$entry" ]; then continue; fi
    case "$entry" in
      # Only links to other units, by name: nothing is read from them as settings.
      *.wants | *.requires | *.upholds) continue ;;
    esac
    if ! find -L "$entry" ! -type d -print0 > "$TMPDIR/files"; then
      echo "$entry: could not be read through" >> "$bad"
    fi
    while IFS= read -r -d "" file; do
      # Where the file really is: in the store (so the same on the machine as here), or /dev/null (a unit
      # masked). Anything else is not known here.
      real=$(realpath -e -- "$file" 2>/dev/null || true)
      case "$real" in
        /dev/null) ;;
        /nix/store/*)
          if [ ! -f "$real" ] || [ ! -r "$real" ]; then
            echo "$file: not a file that can be read" >> "$bad"
          elif grep -q -E '^[[:space:]]*Delegate' -- "$real"; then
            echo "$file: $(grep -E '^[[:space:]]*Delegate' -- "$real" | head -n 1)" >> "$bad"
          fi
          ;;
        *) echo "$file: a link out of the store, or to nothing: what it says on the machine is not known here" >> "$bad" ;;
      esac
    done < "$TMPDIR/files"
  done

  if [ -s "$foreign" ]; then
    {
      echo "torq-agent: the agent's units are only what TorqOS's own module writes: each of torq-agent@<person>.service"
      echo "and its socket is the one file this configuration makes, with no drop-in and"
      echo "no other unit of those names, whoever brings it (a package in systemd.packages, an alias, a unit asked for"
      echo "as a drop-in). Any of them could change who the agent runs as, what it runs, its credentials or its"
      echo "sandbox. Found in the units as built:"
      cat "$foreign"
      if [ -n "$generic" ]; then
        echo "A drop-in for every service or socket (service.d, socket.d), or for every unit whose name begins torq-,"
        echo "is a drop-in for the agent's units too. TorqOS refuses it whatever it sets, because any line of it could"
        echo "change the agent, and nothing turns this check off. The package that brings it can say the same in a"
        echo "drop-in for its own unit (<its unit>.service.d), which is not refused; or leave the package out of"
        echo "systemd.packages."
      fi
    } >&2
  fi
  if [ -s "$changed" ]; then
    {
      echo "torq-agent: the agent's units say only what TorqOS's own module writes, word for word: who the agent runs"
      echo "as, what it runs, its credentials, its sandbox, its socket. Another part of this configuration changes one"
      echo "(a setting under systemd.services, systemd.sockets or systemd.units for that name). Found in the units as"
      echo "built:"
      cat "$changed"
    } >&2
  fi
  if [ -s "$bad" ]; then
    {
      echo "torq-agent: no Delegate or DelegateSubgroup in any torq-agent@ unit, whoever brings it: a unit file or a"
      echo "drop-in from a package (systemd.packages) counts as this configuration's own does. The agent's service ends"
      echo "everything the engine started by ending every process in the unit's control group, which only root may"
      echo "move a process into or out of. Found in the units as built:"
      cat "$bad"
    } >&2
  fi
  if [ -s "$foreign" ] || [ -s "$changed" ] || [ -s "$bad" ]; then exit 1; fi
  touch $out
''
