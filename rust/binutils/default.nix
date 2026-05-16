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
        tool = "nm";
        name = "debug-syms";
      }
      {
        tool = "readelf";
        name = "string-dump";
      }
      {
        tool = "readelf";
        name = "hex-dump";
      }
      {
        tool = "readelf";
        name = "headers-combined";
      }
      {
        tool = "readelf";
        name = "header-sections";
      }
      {
        tool = "readelf";
        name = "header-program";
      }
      {
        tool = "readelf";
        name = "hex-dump-data";
      }
      {
        tool = "cxxfilt";
        name = "no-params";
      }
      {
        tool = "addr2line";
        name = "multiple";
      }
      {
        tool = "nm";
        name = "print-file-name-posix";
      }
      {
        tool = "nm";
        name = "just-symbols-with-prefix";
      }
      {
        tool = "nm";
        name = "radix-hex-print-file-name";
      }
      {
        tool = "strings";
        name = "radix-decimal";
      }
      {
        tool = "strings";
        name = "radix-octal";
      }
      {
        tool = "nm";
        name = "multiple-files";
      }
      {
        tool = "size";
        name = "multiple-files";
      }
      {
        tool = "readelf";
        name = "multiple-files";
      }
      {
        tool = "objdump";
        name = "multiple-files";
      }
      {
        tool = "strings";
        name = "multiple-files";
      }
      {
        tool = "addr2line";
        name = "inlines";
      }
      {
        tool = "nm";
        name = "no-recurse-limit";
      }
      {
        tool = "nm";
        name = "reverse-numeric";
      }
      {
        tool = "cxxfilt";
        name = "style";
      }
      {
        tool = "cxxfilt";
        name = "no-recurse-limit";
      }
      {
        tool = "nm";
        name = "format-bsd";
      }
      {
        tool = "nm";
        name = "special-syms";
      }
      {
        tool = "nm";
        name = "extern-only-posix";
      }
      {
        tool = "nm";
        name = "undefined-only-posix";
      }
      {
        tool = "readelf";
        name = "notes-with-sections";
      }
      {
        tool = "readelf";
        name = "header-notes";
      }
      {
        tool = "addr2line";
        name = "basenames";
      }
      {
        tool = "readelf";
        name = "version-info";
      }
      {
        tool = "nm";
        name = "line-numbers";
      }
      {
        tool = "nm";
        name = "all-with-prefix";
      }
      {
        tool = "readelf";
        name = "program-wide";
      }
      {
        tool = "readelf";
        name = "dynamic-arch";
      }
      {
        tool = "readelf";
        name = "groups-wide";
      }
      {
        tool = "readelf";
        name = "notes-symbols";
      }
      {
        tool = "readelf";
        name = "sections-arch-wide";
      }
      {
        tool = "nm";
        name = "no-weak";
      }
      {
        tool = "nm";
        name = "posix-decimal";
      }
      {
        tool = "nm";
        name = "posix-octal";
      }
      {
        tool = "nm";
        name = "print-size";
      }
      {
        tool = "nm";
        name = "target-noop";
      }
      {
        tool = "ar";
        name = "offsets";
      }
      {
        tool = "objdump";
        name = "section-headers-wide";
      }
      {
        tool = "readelf";
        name = "unwind";
      }
      {
        tool = "readelf";
        name = "archive-index-not-archive";
      }
      {
        tool = "readelf";
        name = "string-dump-text";
      }
      {
        tool = "readelf";
        name = "hex-dump-text";
      }
      {
        tool = "readelf";
        name = "string-dump-data";
      }
      {
        tool = "addr2line";
        name = "functions";
      }
      {
        tool = "addr2line";
        name = "addresses";
      }
      {
        tool = "addr2line";
        name = "pretty-functions";
      }
      {
        tool = "addr2line";
        name = "addresses-functions-pretty";
      }
      {
        tool = "strings";
        name = "print-file-name";
      }
      {
        tool = "nm";
        name = "archive";
      }
      {
        tool = "nm";
        name = "print-armap";
      }
      {
        tool = "objdump";
        name = "archive";
      }
      {
        tool = "size";
        name = "common";
      }
      {
        tool = "cxxfilt";
        name = "strip-underscore";
      }
      {
        tool = "nm";
        name = "numeric-extern";
      }
      {
        tool = "nm";
        name = "last-wins-sort";
      }
      {
        tool = "readelf";
        name = "use-dynamic";
      }
      {
        tool = "objdump";
        name = "disassemble-section";
      }
      {
        tool = "size";
        name = "sysv-hex";
      }
      {
        tool = "size";
        name = "sysv-octal";
      }
      {
        tool = "nm";
        name = "size-sort";
      }
      {
        tool = "objdump";
        name = "headers-section-filter";
      }
      {
        tool = "objdump";
        name = "syms-section-filter";
      }
      {
        tool = "objdump";
        name = "no-show-raw-insn";
      }
      {
        tool = "objdump";
        name = "intel-syntax";
      }
      {
        tool = "nm";
        name = "no-demangle";
      }
      {
        tool = "nm";
        name = "quiet";
      }
      {
        tool = "objdump";
        name = "intel-syntax-bundled";
      }
      {
        tool = "objdump";
        name = "disassemble-zeroes";
      }
      {
        tool = "objdump";
        name = "section-filter-bundled";
      }
      {
        tool = "objdump";
        name = "start-address";
      }
      {
        tool = "readelf";
        name = "dynamic-program-headers";
      }
      {
        tool = "addr2line";
        name = "out-of-section";
      }
      {
        tool = "readelf";
        name = "notes-wide";
      }
      {
        tool = "objdump";
        name = "intel-mnemonic";
      }
      {
        tool = "objdump";
        name = "headers-syms";
      }
      {
        tool = "objdump";
        name = "disassemble-with-syms";
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
