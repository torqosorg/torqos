{ lib, rustPlatform }:

rustPlatform.buildRustPackage {
  pname = "torq-model-proxy";
  # One place: Cargo.toml (scripts/bump-version sets it).
  version = (lib.importTOML ./Cargo.toml).package.version;
  # This crate and torq-wire (ADR-044: the record's words), nothing else of the repository.
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.difference (lib.fileset.unions [ ./. ../wire ])
      (lib.fileset.unions [ (lib.fileset.maybeMissing ./target) (lib.fileset.maybeMissing ../wire/target) ]);
  };
  cargoRoot = "proxy";
  buildAndTestSubdir = "proxy";
  # Every crate it is built from is named in the lock file with its checksum, and fetched by that.
  cargoLock.lockFile = ./Cargo.lock;

  meta = {
    description = "TorqOS's model proxy: every request an agent makes of its model passes here and is recorded, and the sites an agent may reach are decided here (ADR-057)";
    mainProgram = "torq-model-proxy";
    license = lib.licenses.asl20;
  };
}
