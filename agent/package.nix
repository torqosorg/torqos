{ lib, stdenv, rustPlatform, symlinkJoin, pciutils, usbutils, iproute2, iw, util-linux, kmod, procps, coreutils }:

let
  # The programs torq-mcp's tools run (agent/src/tools.rs), by their store paths, never from PATH: the
  # hardware tools that only report. journalctl and torqcli are the system's own
  # (/run/current-system/sw/bin), matching its journal and its broker. Linux only, as torqcli's filters are.
  toolsEnv = lib.optionalAttrs stdenv.hostPlatform.isLinux {
    TORQ_TOOLS_BIN = "${symlinkJoin { name = "torq-agent-tools"; paths = [ pciutils usbutils iproute2 iw util-linux kmod procps coreutils ]; }}/bin";
  };
in

rustPlatform.buildRustPackage {
  pname = "torq-agent";
  # One place: Cargo.toml (scripts/bump-version sets it).
  version = (lib.importTOML ./Cargo.toml).package.version;
  # This crate and torq-wire (ADR-044: the agent service's words), nothing else of the repository.
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.difference (lib.fileset.unions [ ./. ../wire ])
      (lib.fileset.unions [ (lib.fileset.maybeMissing ./target) (lib.fileset.maybeMissing ../wire/target) ]);
  };
  cargoRoot = "agent";
  buildAndTestSubdir = "agent";
  cargoLock.lockFile = ./Cargo.lock;

  env = toolsEnv;
  # For the dev shell (flake.nix), so `cargo test` there builds the same way.
  passthru = { inherit toolsEnv; };

  meta = {
    description = "TorqOS agent (unprivileged, as a person's torq-agent-<person>): the agent service, the bridge to the person's engine; and torq-mcp, TorqOS's tools for it (reading the machine; a change proposed, never applied)";
    mainProgram = "torq-agent-service";
    license = lib.licenses.asl20;
  };
}
