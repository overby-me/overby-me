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

      # Three of them resolve real workspaces and take them as arguments; the
      # rest are self-contained. Applied by shape rather than by naming which
      # is which, so that a test gaining or losing a fixture is a change to
      # that file alone. Their defaults are the monorepo paths, which is what
      # `nix eval -f` on one of them uses; these are the same sources reached
      # as inputs, so a clone of this directory runs them too.
      #
      # Handed over as paths rather than as the inputs themselves. An input is
      # an attribute set that coerces to its store path, which is enough for a
      # derivation's `src` and not enough for the manifest library: that asks
      # `isPath` to decide how to join, gets false, and builds strings which
      # `pathExists` then answers false for - so a workspace reads as having no
      # targets at all rather than failing as a type error. The context has to
      # go with it, as it does for the graph JSON in the ninja library: a store
      # path is what this already is, and a path cannot carry the reference.
      inputPath = i: /. + (builtins.unsafeDiscardStringContext i.outPath);
      sources = {
        wclipSrc = inputPath pkgs.inputs.wclip;
        xzSrc = inputPath pkgs.inputs.oxidized-xz;
      };
      run = n: let
        t = import (./tests + "/${n}.nix");
      in
        if builtins.isFunction t
        then t sources
        else t;

      results = map (n: "${n}: ${run n}") names;
    in
      pkgs.writeText "cargo-lib-tests" (builtins.concatStringsSep "\n" results);

    # End-to-end: wclip built with per-crate derivations, binary smoke test.
    cargo-build-wclip = pkgs: let
      drv = pkgs.lib.buildCargoProject {
        src = pkgs.inputs.wclip;
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
        src = pkgs.inputs.wclip;
        index = ./index;
        runTests = true;
      })
      .tests."rust-wclip";

    # End-to-end: xz with native liblzma linking through crateOverrides and
    # dev-deps excluded (criterion must not be built).
    cargo-build-xz = pkgs: let
      drv = pkgs.lib.buildCargoProject {
        src = pkgs.inputs.oxidized-xz;
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
