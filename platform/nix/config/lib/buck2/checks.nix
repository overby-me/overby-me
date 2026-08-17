# Flakelight module: checks for the buck2 library. Imported explicitly from
# flake.nix (the platform/nix/config/lib autoloader only routes default.nix).
#
# Run one: nix build .#checks.x86_64-linux.buck2-build-cpp
# (never `nix flake check`, see the repo rules)
{
  checks = {
    # Pure eval unit tests (label/cell resolution, load phase, analysis).
    buck2-lib = pkgs: let
      names = ["labels" "load" "analysis"];
      results = map (n: "${n}: ${import (./tests + "/${n}.nix")}") names;
    in
      pkgs.writeText "buck2-lib-tests" (builtins.concatStringsSep "\n" results);

    # End-to-end: build the no_prelude C++ binary (one derivation per action,
    # no IFD) and run it.
    buck2-build-cpp = pkgs: let
      drv = pkgs.lib.buildBuck2Project {
        src = ./tests/fixtures/no_prelude;
        target = "//cpp/hello_world:main";
      };
    in
      pkgs.runCommand "buck2-build-cpp" {} ''
        ${drv}/main > $out
        grep -q "Hello from C++!" $out
      '';

    # End-to-end: build the no_prelude C++ shared library and check its symbol.
    buck2-build-cpp-library = pkgs: let
      drv = pkgs.lib.buildBuck2Project {
        src = ./tests/fixtures/no_prelude;
        target = "//cpp/library:library";
      };
    in
      pkgs.runCommand "buck2-build-cpp-library" {} ''
        test -f ${drv}/lib.so
        grep -q print_hello ${drv}/lib.so
        touch $out
      '';

    # End-to-end: build the no_prelude Rust binary and run it.
    buck2-build-rust = pkgs: let
      drv = pkgs.lib.buildBuck2Project {
        src = ./tests/fixtures/no_prelude;
        target = "//rust:main";
      };
    in
      pkgs.runCommand "buck2-build-rust" {} ''
        ${drv}/main > $out
        grep -q "Hello from Rust!" $out
      '';

    # End-to-end via the opt-in IFD analysis path (analysis runs in a cached
    # derivation, not at eval time). Same output; guards the IFD path.
    buck2-build-cpp-ifd = pkgs: let
      drv = pkgs.lib.buildBuck2Project {
        src = ./tests/fixtures/no_prelude;
        target = "//cpp/hello_world:main";
        ifdAnalysis = true;
      };
    in
      pkgs.runCommand "buck2-build-cpp-ifd" {} ''
        ${drv}/main > $out
        grep -q "Hello from C++!" $out
      '';

    # End-to-end: build the no_prelude Go binary and run it. Exercises the full
    # toolchain dance: download_file (Go tarball via fetchurl), write (unpack
    # script), extract, symlink, then `go build`. Network-heavy (fetches the Go
    # toolchain) but pure (fixed sha256 in the Starlark source).
    buck2-build-go = pkgs: let
      drv = pkgs.lib.buildBuck2Project {
        src = ./tests/fixtures/no_prelude;
        target = "//go:main";
      };
    in
      pkgs.runCommand "buck2-build-go" {} ''
        ${drv}/main > $out
        grep -q "Hello from Go!" $out
      '';
  };
}
