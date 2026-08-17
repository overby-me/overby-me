# Flakelight module: checks for the cargo library. Imported explicitly from
# flake.nix (the platform/nix/config/lib autoloader only routes default.nix, on purpose:
# checks are not lib content).
#
# Run one: nix build .#checks.x86_64-linux.cargo-lib
# (never `nix flake check`, see the repo rules)
{
  checks = {
    # Pure eval unit tests: importing a test file throws on failure, so
    # instantiating this derivation is the assertion.
    cargo-lib = pkgs: let
      names = ["semver" "cfg" "lock" "index" "manifest" "shapes" "profile" "resolve" "resolve-real" "patch"];
      results = map (n: "${n}: ${import (./tests + "/${n}.nix")}") names;
    in
      pkgs.writeText "cargo-lib-tests" (builtins.concatStringsSep "\n" results);

    # End-to-end: wclip built with per-crate derivations, binary smoke test.
    cargo-build-wclip = pkgs: let
      drv = pkgs.lib.buildCargoProject {
        src = ../../../../../dev/wclip;
        index = ./index;
      };
    in
      pkgs.runCommand "cargo-build-wclip" {} ''
        ${drv}/bin/wclip --version > $out
        grep -q "wclip 0.1.0" $out
      '';

    # End-to-end: compile and run wclip's unit tests through runTests.
    cargo-test-wclip = pkgs:
      (pkgs.lib.buildCargoProject {
        src = ../../../../../dev/wclip;
        index = ./index;
        runTests = true;
      })
      .tests."rust-wclip";

    # End-to-end: xz with native liblzma linking through crateOverrides and
    # dev-deps excluded (criterion must not be built).
    cargo-build-xz = pkgs: let
      drv = pkgs.lib.buildCargoProject {
        src = ../../../../../safety/oxidized/xz;
        index = ./index;
        bins = ["xz"];
        crateOverrides.liblzma-sys = {
          nativeBuildInputs = [pkgs.pkg-config];
          buildInputs = [pkgs.xz];
        };
      };
    in
      pkgs.runCommand "cargo-build-xz" {} ''
        echo hello | ${drv}/bin/xz | ${drv}/bin/xz -d > $out
        grep -q hello $out
      '';
  };
}
