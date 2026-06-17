# Builds a Deno project using dynamically-fetched npm dependencies.
#
# Dependencies are fetched individually from the npm registry using integrity
# hashes from deno.lock (see fetchDenoDeps.nix), so no manual output hash
# maintenance is needed.
#
# Exposed via perSystemLib as lib.buildDenoProject in pkgs.
#
# Usage in a package definition:
#   packages.my-app = { lib, ... }:
#     lib.buildDenoProject {
#       pname = "my-app";
#       src = ./my-app;
#       buildCommand = "deno run -A npm:@rsbuild/core build";
#       installPhase = "cp -r dist $out";
#     };
{
  lib,
  stdenvNoCC,
  deno,
  fetchurl,
  jq,
  writeText,
}: let
  fetchDenoDeps = import ./fetchDenoDeps.nix {inherit lib stdenvNoCC fetchurl jq writeText;};
in
  {
    src,
    lockFile ? src + "/deno.lock",
    buildCommand,
    installPhase,
    pname ? "deno-project",
    version ? "0.0.0",
    nativeBuildInputs ? [],
    meta ? {},
    ...
  }: let
    deps = fetchDenoDeps {inherit lockFile;};
  in
    stdenvNoCC.mkDerivation {
      inherit pname version src installPhase meta;

      nativeBuildInputs = [deno] ++ nativeBuildInputs;

      buildPhase = ''
        runHook preBuild

        # Create writable copy of pre-fetched Deno cache
        cp -r ${deps} $TMPDIR/deno-cache
        chmod -R u+w $TMPDIR/deno-cache
        export DENO_DIR=$TMPDIR/deno-cache
        export HOME=$TMPDIR

        # Install dependencies from cache (creates node_modules/)
        deno install --frozen

        ${buildCommand}

        runHook postBuild
      '';
    }
