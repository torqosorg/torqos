{ lib, stdenv, rustPlatform, symlinkJoin, curl, pciutils, usbutils, iproute2, iw, util-linux, kmod, procps, coreutils }:

let
  # The programs doctor mode runs (agent/src/tools.rs), by their store paths, never from PATH: curl
  # for the model (agent/src/model.rs), the hardware tools that only report. journalctl and torqcli
  # are the system's own (/run/current-system/sw/bin), matching its journal and its broker.
  # Linux only, as torqcli's filters are.
  doctorEnv = lib.optionalAttrs stdenv.hostPlatform.isLinux {
    TORQ_CURL = lib.getExe curl;
    TORQ_DOCTOR_BIN = "${symlinkJoin { name = "torq-doctor-tools"; paths = [ pciutils usbutils iproute2 iw util-linux kmod procps coreutils ]; }}/bin";
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

  env = doctorEnv;
  # For the dev shell (flake.nix), so `cargo test` there builds the same way.
  passthru = { inherit doctorEnv; };

  meta = {
    description = "TorqOS agent (unprivileged, as a person's torq-agent-<person>): the agent service, the bridge to the person's engine; torq-mcp; and, until it is removed, doctor mode's read-only tools and a change proposed in propose mode, never applied";
    mainProgram = "torq-doctor";
    license = lib.licenses.asl20;
  };
}
