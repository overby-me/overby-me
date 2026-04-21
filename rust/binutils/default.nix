{
  packages = {
    rust-binutils = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-binutils";
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
          # Create symlinks for all binutils tools (multicall binary)
          for tool in ar ranlib nm objdump readelf objcopy strings size addr2line c++filt strip as ld; do
            ln -s $out/bin/rust-binutils $out/bin/$tool
          done
        '';

        meta = {
          description = "GNU binutils-compatible binary utilities written in Rust";
          license = lib.licenses.mit;
          mainProgram = "ar";
        };
      };

    rust-binutils-dev = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-binutils-dev";
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
          # Create symlinks for all binutils tools (multicall binary)
          for tool in ar ranlib nm objdump readelf objcopy strings size addr2line c++filt strip as ld; do
            ln -s $out/bin/rust-binutils $out/bin/$tool
          done
        '';

        meta = {
          description = "GNU binutils-compatible binary utilities written in Rust (dev build, fast compile)";
          license = lib.licenses.mit;
          mainProgram = "ar";
        };
      };
  };

  checks = let
    testDefs = [
      {
        tool = "nm";
        name = "basic";
      }
      {
        tool = "nm";
        name = "extern-only";
      }
      {
        tool = "strings";
        name = "basic";
      }
      {
        tool = "strings";
        name = "object";
      }
      {
        tool = "size";
        name = "basic";
      }
      {
        tool = "size";
        name = "sysv";
      }
      {
        tool = "cxxfilt";
        name = "basic";
      }
      {
        tool = "cxxfilt";
        name = "multiple";
      }
      {
        tool = "readelf";
        name = "file-header";
      }
      {
        tool = "readelf";
        name = "sections";
      }
      {
        tool = "objdump";
        name = "headers";
      }
      {
        tool = "objdump";
        name = "disassemble";
      }
      {
        tool = "ar";
        name = "create-list";
      }
      {
        tool = "addr2line";
        name = "basic";
      }
      {
        tool = "nm";
        name = "no-sort";
      }
      {
        tool = "nm";
        name = "undefined-only";
      }
      {
        tool = "strings";
        name = "min-length";
      }
      {
        tool = "ar";
        name = "extract";
      }
      {
        tool = "readelf";
        name = "program-headers";
      }
      {
        tool = "readelf";
        name = "symbols";
      }
      {
        tool = "objdump";
        name = "syms";
      }
      {
        tool = "objdump";
        name = "relocs";
      }
      {
        tool = "size";
        name = "totals";
      }
      {
        tool = "cxxfilt";
        name = "nested";
      }
    ];
  in
    builtins.listToAttrs (map (t: {
        name = "rust-binutils-test-${t.tool}-${t.name}";
        value = pkgs:
          import ./testsuite.nix {
            inherit pkgs;
            inherit (t) tool name;
          };
      })
      testDefs);
}
