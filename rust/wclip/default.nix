{lib, ...}: {
  packages = {
    rust-wclip = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-wclip";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        meta = {
          description = "An xclip-style Wayland clipboard tool written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/wclip";
          license = lib.licenses.mit;
          mainProgram = "wclip";
          platforms = lib.platforms.linux;
        };
      };

    rust-wclip-dev = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-wclip-dev";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        buildType = "debug";

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
    # The wire-protocol roundtrip tests run during the package build itself.
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
