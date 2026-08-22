# Every name below is local: `default`, `dev`, `test-version`. The directory
# this sits in turns them into `wclip`, `wclip-dev`, `wclip-test-version`.
{lib, ...}: let
  crate = {
    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [
        ./Cargo.toml
        ./Cargo.lock
        ./src
      ];
    };

    meta = {
      homepage = "https://tangled.org/overby.me/overby.me/tree/main/dev/wclip";
      license = lib.licenses.mit;
      mainProgram = "wclip";
      platforms = lib.platforms.linux;
    };
  };

  # Command-line behaviour exercised in the sandbox (no compositor needed). The
  # per-crate cargo builder does not run `cargo test` targets yet (see
  # platform/nix/lib/lib/cargo/PLAN.md), so these carry the wire-protocol tests.
  testNames = [
    "version"
    "version-format"
    "help"
    "help-mentions-clipboard"
    "help-exit-zero"
    "invalid-option"
    "secondary-selection"
    "bad-loops"
    "missing-argument"
    "no-display"
    "selection-abbreviation"
  ];
in {
  packages = {
    default = {lib, ...}:
      lib.buildCargoProject (crate
        // {
          pname = "wclip";
          meta = crate.meta // {description = "An xclip-style Wayland clipboard tool written in Rust";};
        });

    dev = {lib, ...}:
      lib.buildCargoProject (crate
        // {
          pname = "wclip-dev";
          release = false;
          meta = crate.meta // {description = "An xclip-style Wayland clipboard tool written in Rust (dev build, fast compile)";};
        });
  };

  checks = lib.listToAttrs (
    map (name: {
      name = "test-${name}";
      value = pkgs: import ./testsuite.nix {inherit pkgs name;};
    })
    testNames
  );
}
