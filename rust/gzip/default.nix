{
  packages = {
    rust-gzip = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-gzip";
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
        postInstall = ''
          ln -s $out/bin/gzip $out/bin/gunzip
          ln -s $out/bin/gzip $out/bin/zcat
        '';

        meta = {
          description = "A GNU gzip-compatible compression tool written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/gzip";
          license = lib.licenses.mit;
          mainProgram = "gzip";
        };
      };

    rust-gzip-dev = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-gzip-dev";
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

        postInstall = ''
          ln -s $out/bin/gzip $out/bin/gunzip
          ln -s $out/bin/gzip $out/bin/zcat
        '';

        meta = {
          description = "A GNU gzip-compatible compression tool written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/gzip";
          license = lib.licenses.mit;
          mainProgram = "gzip";
        };
      };
  };

  checks = let
    testNames = [
      "gzip-env"
      "helin-segv"
      "help-version"
      "hufts"
      "keep"
      "list"
      "list-big"
      "memcpy-abuse"
      "mixed"
      "null-suffix-clobber"
      "pipe-output"
      "reference"
      "reproducible"
      "stdin"
      "synchronous"
      "timestamp"
      "trailing-nul"
      "two-files"
      "unpack-invalid"
      "unpack-valid"
      "upper-suffix"
      "write-error"
      "z-suffix"
      "zdiff"
      "zgrep-abuse"
      "zgrep-binary"
      "zgrep-context"
      "zgrep-f"
      "zgrep-signal"
      "znew-k"
    ];
  in
    builtins.listToAttrs (map (name: {
        name = "rust-gzip-test-${name}";
        value = pkgs: import ./testsuite.nix {inherit pkgs name;};
      })
      testNames);
}
