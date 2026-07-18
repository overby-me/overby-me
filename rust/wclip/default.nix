{lib, ...}: {
  packages = {
    rust-wclip = {lib, ...}:
      lib.buildCargoProject {
        pname = "rust-wclip";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        index = ../../nix/lib/cargo/index;

        meta = {
          description = "An xclip-style Wayland clipboard tool written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/wclip";
          license = lib.licenses.mit;
          mainProgram = "wclip";
          platforms = lib.platforms.linux;
        };
      };

    rust-wclip-dev = {lib, ...}:
      lib.buildCargoProject {
        pname = "rust-wclip-dev";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        index = ../../nix/lib/cargo/index;
        release = false;

        meta = {
          description = "An xclip-style Wayland clipboard tool written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/wclip";
          license = lib.licenses.mit;
          mainProgram = "wclip";
          platforms = lib.platforms.linux;
        };
      };
  };

  checks = let
    # Command-line behaviour exercised in the sandbox (no compositor needed).
    # Note: the per-crate cargo builder does not run `cargo test` targets yet
    # (see nix/lib/cargo/PLAN.md), so the in-crate wire-protocol unit tests
    # rely on these checks for coverage until test targets land.
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
  in
    lib.listToAttrs (
      map (name: {
        name = "rust-wclip-test-${name}";
        value = pkgs: import ./testsuite.nix {inherit pkgs name;};
      })
      testNames
    );
}
