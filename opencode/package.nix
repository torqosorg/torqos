# opencode for the agents (ADR-058, `torq agent tools opencode`): the upstream release's own program, pinned by
# its hash, not built here.
#
# Why not nixpkgs' opencode (1.15.10 in nixos-26.05, built from source): it does not start on a processor without
# AVX2 — not even `opencode --version` (SIGILL; the check host's Xeon E5-2690 has AVX and no AVX2). Its runtime,
# bun, is built for AVX2 by default. Upstream releases a "baseline" build for such processors, and that is the
# one taken here: it runs on every x86_64 processor TorqOS runs on, with or without AVX2.
#
# What it is: one program, bun with opencode's code inside it (`bun build --compile`), linked against glibc
# alone. Its interpreter and library path are set to this system's glibc (autoPatchelfHook); it is never
# stripped, which would cut off the code it carries at its end. It looks for ripgrep (`rg`) on its PATH and
# downloads one from GitHub where there is none: the system's is put on its PATH, so it never tries.
{ lib, stdenvNoCC, fetchurl, autoPatchelfHook, makeBinaryWrapper, ripgrep }:

stdenvNoCC.mkDerivation (finalAttrs: {
  pname = "opencode";
  version = "1.15.10";

  src = fetchurl {
    url = "https://github.com/anomalyco/opencode/releases/download/v${finalAttrs.version}/opencode-linux-x64-baseline.tar.gz";
    hash = "sha256-4OBrNSzw82QcZh5ocNpZEnnlrWVG40pfCkqYYdlXuDY=";
  };
  sourceRoot = ".";

  nativeBuildInputs = [ autoPatchelfHook makeBinaryWrapper ];
  dontStrip = true;
  dontConfigure = true;
  dontBuild = true;

  installPhase = ''
    runHook preInstall
    install -Dm755 opencode $out/libexec/opencode/opencode
    makeBinaryWrapper $out/libexec/opencode/opencode $out/bin/opencode --prefix PATH : ${lib.makeBinPath [ ripgrep ]}
    runHook postInstall
  '';

  # The program starts, and says its version (the build's own check that the pin is what it says).
  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    home=$(mktemp -d)
    HOME=$home TMPDIR=$home XDG_CACHE_HOME=$home/cache XDG_DATA_HOME=$home/data XDG_CONFIG_HOME=$home/config XDG_STATE_HOME=$home/state \
      OPENCODE_DISABLE_AUTOUPDATE=1 OPENCODE_DISABLE_MODELS_FETCH=1 \
      $out/bin/opencode --version | grep -Fx ${finalAttrs.version}
    runHook postInstallCheck
  '';

  meta = {
    description = "opencode, the coding agent, as its upstream release builds it for x86_64 processors with or without AVX2";
    homepage = "https://github.com/anomalyco/opencode";
    license = lib.licenses.mit;
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
    platforms = [ "x86_64-linux" ];
    mainProgram = "opencode";
  };
})
