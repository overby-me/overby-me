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
          for tool in ar ranlib nm objdump readelf objcopy strings size addr2line c++filt strip as ld elfedit; do
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
          for tool in ar ranlib nm objdump readelf objcopy strings size addr2line c++filt strip as ld elfedit; do
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
        tool = "nm";
        name = "numeric-sort";
      }
      {
        tool = "nm";
        name = "defined-only";
      }
      {
        tool = "nm";
        name = "reverse-sort";
      }
      {
        tool = "nm";
        name = "radix-decimal";
      }
      {
        tool = "nm";
        name = "print-file-name";
      }
      {
        tool = "size";
        name = "decimal";
      }
      {
        tool = "size";
        name = "octal";
      }
      {
        tool = "size";
        name = "hex";
      }
      {
        tool = "nm";
        name = "radix-octal";
      }
      {
        tool = "nm";
        name = "radix-hex";
      }
      {
        tool = "ar";
        name = "print";
      }
      {
        tool = "addr2line";
        name = "pretty";
      }
      {
        tool = "nm";
        name = "posix";
      }
      {
        tool = "readelf";
        name = "headers-all";
      }
      {
        tool = "readelf";
        name = "notes";
      }
      {
        tool = "readelf";
        name = "relocs";
      }
      {
        tool = "nm";
        name = "just-symbols";
      }
      {
        tool = "readelf";
        name = "dynamic";
      }
      {
        tool = "readelf";
        name = "arch-specific";
      }
      {
        tool = "readelf";
        name = "groups";
      }
      {
        tool = "objdump";
        name = "section-filter";
      }
      {
        tool = "strings";
        name = "radix-hex";
      }
      {
        tool = "cxxfilt";
        name = "no-strip-leading";
      }
      {
        tool = "readelf";
        name = "wide";
      }
      {
        tool = "objdump";
        name = "file-headers";
      }
      {
        tool = "readelf";
        name = "string-dump-missing";
      }
      {
        tool = "readelf";
        name = "histogram";
      }
      {
        tool = "cxxfilt";
        name = "types";
      }
      {
        tool = "addr2line";
        name = "demangle";
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

    # Upstream DejaGnu .exp files to run individually.
    # Each gets a check like: rust-binutils-dejagnu-size
    dejaGnuTests = [
      {
        exp = "cxxfilt.exp";
        minPass = 3;
        maxFail = 0;
      }
      {
        exp = "size.exp";
        minPass = 3;
        maxFail = 0;
      }
      {
        exp = "strings.exp";
        minPass = 1;
        maxFail = 0;
      }
      {
        exp = "nm.exp";
        minPass = 15;
        maxFail = 0;
      }
      {
        exp = "ar.exp";
        minPass = 14;
        maxFail = 0;
      }
      {
        exp = "addr2line.exp";
        minPass = 3;
        maxFail = 0;
      }
      {
        exp = "readelf.exp";
        minPass = 38;
        maxFail = 0;
      }
      {
        exp = "objdump.exp";
        minPass = 32;
        maxFail = 0;
      }
      {
        exp = "objcopy.exp";
        minPass = 120;
        maxFail = 0;
      }
      {
        exp = "compress.exp";
        minPass = 45;
        maxFail = 0;
      }
      {
        exp = "update-section.exp";
        minPass = 6;
        maxFail = 0;
      }
      {
        exp = "elfedit.exp";
        minPass = 6;
        maxFail = 0;
      }
    ];

    customChecks = builtins.listToAttrs (map (t: {
        name = "rust-binutils-test-${t.tool}-${t.name}";
        value = pkgs:
          import ./testsuite.nix {
            inherit pkgs;
            inherit (t) tool name;
          };
      })
      testDefs);

    dejaGnuChecks = builtins.listToAttrs (map (t: let
        baseName = builtins.replaceStrings [".exp"] [""] t.exp;
      in {
        name = "rust-binutils-dejagnu-${baseName}";
        value = pkgs:
          import ./dejagnu-testsuite.nix {
            inherit pkgs;
            expFile = t.exp;
            inherit (t) minPass maxFail;
          };
      })
      dejaGnuTests);

    # Single check that runs ALL upstream .exp files (informational, always passes)
    dejaGnuAll = {
      rust-binutils-dejagnu-all = pkgs:
        import ./dejagnu-testsuite.nix {
          inherit pkgs;
        };
    };
  in
    customChecks // dejaGnuChecks // dejaGnuAll;
}
