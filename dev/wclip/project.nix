# A project module, applied to its own identity by the workspace.
#
# Every name below is local: `default`, `dev`, `test-version`. The project
# turns them into `wclip`, `wclip-dev`, `wclip-test-version`, from where this
# directory sits, so nothing here can spell a name that lands in another
# project's namespace and no prefix has to be remembered.
project: {lib, ...}: let
  crate = {
    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [
        ./Cargo.toml
        ./Cargo.lock
        ./src
      ];
    };

    index = ../../platform/nix/lib/cargo/index;

    meta = {
      homepage = "https://tangled.org/overby.me/overby.me/tree/main/${project.path}";
      license = lib.licenses.mit;
      mainProgram = "wclip";
      platforms = lib.platforms.linux;
    };
  };

  # Command-line behaviour exercised in the sandbox (no compositor needed).
  # Note: the per-crate cargo builder does not run `cargo test` targets yet
  # (see platform/nix/lib/cargo/PLAN.md), so the in-crate wire-protocol unit
  # tests rely on these checks for coverage until test targets land.
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
  packages = project.names {
    default = {lib, ...}:
      lib.buildCargoProject (crate
        // {
          # The crate keeps its own name: pname is resolved against
          # Cargo.lock, and this names a target rather than a crate.
          pname = "rust-wclip";
          meta = crate.meta // {description = "An xclip-style Wayland clipboard tool written in Rust";};
        });

    dev = {lib, ...}:
      lib.buildCargoProject (crate
        // {
          pname = "rust-wclip-dev";
          release = false;
          meta = crate.meta // {description = "An xclip-style Wayland clipboard tool written in Rust (dev build, fast compile)";};
        });
  };

  checks =
    project.names
    (lib.listToAttrs (
      map (name: {
        name = "test-${name}";
        value = pkgs: import ./testsuite.nix {inherit pkgs name;};
      })
      testNames
    ));
}
