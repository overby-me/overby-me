{lib, ...}: {
  packages = {
    oxidized-bzip2 = {lib, ...}:
      lib.buildCargoProject {
        pname = "rust-bzip2";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        index = ../../../platform/nix/config/lib/cargo/index;

        rootAttrs.postInstall = ''
          ln -s $out/bin/bzip2 $out/bin/bunzip2
          ln -s $out/bin/bzip2 $out/bin/bzcat
        '';

        meta = {
          description = "A bzip2-compatible compression tool written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/bzip2";
          license = lib.licenses.mit;
          mainProgram = "bzip2";
          platforms = lib.platforms.linux;
        };
      };

    oxidized-bzip2-dev = {lib, ...}:
      lib.buildCargoProject {
        pname = "rust-bzip2-dev";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        index = ../../../platform/nix/config/lib/cargo/index;

        release = false;

        rootAttrs.postInstall = ''
          ln -s $out/bin/bzip2 $out/bin/bunzip2
          ln -s $out/bin/bzip2 $out/bin/bzcat
        '';

        meta = {
          description = "A bzip2-compatible compression tool written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/bzip2";
          license = lib.licenses.mit;
          mainProgram = "bzip2";
          platforms = lib.platforms.linux;
        };
      };
  };

  checks = let
    testNames = [
      "compress-1"
      "compress-2"
      "compress-3"
      "decompress-1"
      "decompress-2"
      "decompress-3"
      "roundtrip-1"
      "roundtrip-2"
      "roundtrip-3"
      "roundtrip-4"
      "roundtrip-5"
      "roundtrip-6"
      "roundtrip-7"
      "roundtrip-8"
      "roundtrip-9"
      "roundtrip-text"
      "roundtrip-binary"
      "integrity"
      "stdin-stdout"
      "symlinks"
      "keep"
      "force-overwrite"
      "empty"
      "large"
      "bad-input"
    ];
  in
    lib.listToAttrs (
      map (name: {
        name = "oxidized-bzip2-test-${name}";
        value = pkgs: import ./testsuite.nix {inherit pkgs name;};
      })
      testNames
    );
}
