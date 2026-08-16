{lib, ...}: {
  packages = {
    oxidized-gzip = {lib, ...}:
      lib.buildCargoProject {
        pname = "rust-gzip";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        index = ../../../platform/nix/lib/cargo/index;
        rootAttrs.postInstall = ''
          ln -s $out/bin/gzip $out/bin/gunzip
          ln -s $out/bin/gzip $out/bin/zcat
        '';

        meta = {
          description = "A GNU gzip-compatible compression tool written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/gzip";
          license = lib.licenses.mit;
          mainProgram = "gzip";
          platforms = lib.platforms.linux;
        };
      };

    oxidized-gzip-dev = {lib, ...}:
      lib.buildCargoProject {
        pname = "rust-gzip-dev";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        index = ../../../platform/nix/lib/cargo/index;
        release = false;

        rootAttrs.postInstall = ''
          ln -s $out/bin/gzip $out/bin/gunzip
          ln -s $out/bin/gzip $out/bin/zcat
        '';

        meta = {
          description = "A GNU gzip-compatible compression tool written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/gzip";
          license = lib.licenses.mit;
          mainProgram = "gzip";
          platforms = lib.platforms.linux;
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
    lib.listToAttrs (
      map (name: {
        name = "oxidized-gzip-test-${name}";
        value = pkgs: import ./testsuite.nix {inherit pkgs name;};
      })
      testNames
    );
}
