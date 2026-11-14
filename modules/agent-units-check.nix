# The agent's units, as built: only what this module writes, and nothing delegated (modules/agent.nix;
# KNOWN-GAPS 6zzo).
#
# The configuration's own checks (agent.nix's assertions) read `systemd.units`: what this configuration writes.
# A unit file or drop-in that a package brings (`systemd.packages`: its lib/systemd/system and
# etc/systemd/system) is not there. It is a file inside a built package, and evaluation cannot read it without
# building the package first. So this reads the finished directory instead, the one NixOS makes
# /etc/systemd/system from, where the packages' files and the configuration's are already put together. It is
# a build of its own that the system's build depends on (`system.checks`): a system that fails it is not built,
# so never switched to.
#
# `units`: that directory. `expected`: the agent's units as the module writes them, each unit's name and the one
# file in the store it must be (`systemd.units.<name>.unit`, the derivation NixOS makes from the unit's text).
#
# Two rules, each said apart in the build's log.
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
#    - nothing else is named as the agent's units are (`torq-agent@…`, `torq-agent-key@…`: another instance,
#      the template, a timer or a path unit of that name), but a mask (a link that ends at /dev/null: it can
#      only keep a unit from starting);
#    - none of the agent's units has a drop-in directory (`<unit>.d`, the template's `torq-agent@.service.d`),
#      and none of the directories systemd also applies to them is there: `service.d` and `socket.d` (every
#      service, every socket), `torq-.service.d`, `torq-agent-.service.d` and the sockets' (the name cut at a
#      dash). The module writes no drop-in, so any that is there is someone else's: NixOS's `overrides.conf`
#      included, which is there only when another file is the unit, or when the configuration asks for its unit
#      as a drop-in.
#    Not read: `<unit>.wants`, `.requires` and `.upholds`, which name other units and set nothing in this one.
#
# 2. Nothing delegated (as before): a line that sets a key beginning with `Delegate` (Delegate,
#    DelegateSubgroup) in any `torq-agent@` unit file or drop-in directory, in `torq-.service.d` and in
#    `service.d`, and a file there that is not in the store (a link out of it, or to nothing: what it would say
#    on the machine is not known here; /dev/null, a masked unit, is let by). Rule 1 leaves this only the
#    module's own files to find it in; it stays, a second reading of the same directory.
{ pkgs }:

{ units, expected }:
pkgs.runCommand "torq-agent-units-its-own"
  {
    inherit units;
    expected = pkgs.lib.concatStringsSep "\n" (pkgs.lib.mapAttrsToList (name: file: "${name} ${file}") expected);
  } ''
  # Rule 1: whose the files are.
  foreign=$TMPDIR/foreign
  : > "$foreign"
  declare -A want
  while IFS=' ' read -r name file; do
    if [ -n "$name" ]; then want[$name]=$file; fi
  done <<< "$expected"
  for entry in "$units"/*; do
    if [ ! -e "$entry" ] && [ ! -L "$entry" ]; then continue; fi
    name=''${entry##*/}
    case "$name" in
      # Only links to other units, by name: nothing is read from them as settings.
      *.wants | *.requires | *.upholds) continue ;;
      service.d | socket.d | torq-.service.d | torq-.socket.d | torq-agent-.service.d | torq-agent-.socket.d)
        echo "$name: a drop-in directory systemd also applies to the agent's units; this configuration writes none" >> "$foreign"
        continue
        ;;
      torq-agent@* | torq-agent-key@*) ;;
      *) continue ;;
    esac
    case "$name" in
      *.d)
        echo "$name: a drop-in directory of the agent's units; this configuration writes none (holding: $(ls -A -- "$entry" 2>/dev/null | tr '\n' ' '))" >> "$foreign"
        continue
        ;;
    esac
    if [ -n "''${want[$name]+set}" ]; then
      if [ ! -L "$entry" ]; then
        echo "$name: not the unit this configuration writes (a file of its own here, not a link to ''${want[$name]})" >> "$foreign"
      elif [ "$(readlink -- "$entry")" != "''${want[$name]}" ]; then
        echo "$name: not the unit this configuration writes (it is $(readlink -- "$entry"), not ''${want[$name]})" >> "$foreign"
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
  for entry in "$units"/torq-agent@* "$units"/torq-.service.d "$units"/service.d; do
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
      echo "torq-agent: the agent's units are only what TorqOS's own module writes: each of torq-agent@<person>.service,"
      echo "its socket and torq-agent-key@<person>.service the one file this configuration makes, with no drop-in and"
      echo "no other unit of those names, whoever brings it (a package in systemd.packages, an alias, a unit asked for"
      echo "as a drop-in). Any of them could change who the agent runs as, what it runs, its credentials or its"
      echo "sandbox. Found in the units as built:"
      cat "$foreign"
    } >&2
  fi
  if [ -s "$bad" ]; then
    {
      echo "torq-agent: no Delegate or DelegateSubgroup in any torq-agent@ unit, whoever brings it: a unit file or a"
      echo "drop-in from a package (systemd.packages) counts as this configuration's own does. The broker takes the"
      echo "unit's control group for the resident agent, which only root may move a process into (a second line"
      echo "behind the resident account's uid). Found in the units as built:"
      cat "$bad"
    } >&2
  fi
  if [ -s "$foreign" ] || [ -s "$bad" ]; then exit 1; fi
  touch $out
''
