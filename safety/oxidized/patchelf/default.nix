{lib, ...}: {
  packages = {
    oxidized-patchelf = {lib, ...}:
      lib.buildCargoProject {
        pname = "rust-patchelf";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        index = ../../../platform/nix/lib/cargo/index;

        meta = {
          description = "A patchelf-compatible ELF binary patching tool written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/patchelf";
          license = lib.licenses.mit;
          mainProgram = "patchelf";
          platforms = lib.platforms.linux;
        };
      };

    oxidized-patchelf-dev = {lib, ...}:
      lib.buildCargoProject {
        pname = "rust-patchelf-dev";

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

        meta = {
          description = "A patchelf-compatible ELF binary patching tool written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/patchelf";
          license = lib.licenses.mit;
          mainProgram = "patchelf";
          platforms = lib.platforms.linux;
        };
      };
  };

  checks = let
    # Upstream src_TESTS list from patchelf 0.15.2 tests/Makefile.am
    srcTests = [
      "plain-fail"
      "plain-run"
      "shrink-rpath"
      "set-interpreter-short"
      "set-interpreter-long"
      "set-rpath"
      "add-rpath"
      "no-rpath"
      "big-dynstr"
      "set-rpath-library"
      "soname"
      "shrink-rpath-with-allowed-prefixes"
      "set-rpath-rel-map"
      "force-rpath"
      "plain-needed"
      "output-flag"
      "too-many-strtab"
      "no-rpath-pie-powerpc"
      "build-id"
      "invalid-elf"
      "endianness"
      "contiguous-note-sections"
      "no-gnu-hash"
      "grow-file"
      "no-dynamic-section"
      "args-from-file"
      "basic-flags"
      "set-empty-rpath"
      "phdr-corruption"
      "replace-needed"
      "replace-add-needed"
      "add-debug-tag"
    ];
    # no_rpath_arch_TESTS — each is a symlink to no-rpath-prebuild.sh
    noRpathArchTests = [
      "no-rpath-amd64"
      "no-rpath-armel"
      "no-rpath-armhf"
      "no-rpath-hurd-i386"
      "no-rpath-i386"
      "no-rpath-ia64"
      "no-rpath-kfreebsd-amd64"
      "no-rpath-kfreebsd-i386"
      "no-rpath-mips"
      "no-rpath-mipsel"
      "no-rpath-powerpc"
      "no-rpath-s390"
      "no-rpath-sh4"
      "no-rpath-sparc"
    ];
    testNames = srcTests ++ noRpathArchTests;
  in
    lib.listToAttrs (
      map (name: {
        name = "oxidized-patchelf-test-${name}";
        value = pkgs: import ./testsuite.nix {inherit pkgs name;};
      })
      testNames
    );
}
