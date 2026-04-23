# Run a single test from the upstream patchelf test suite against rust-patchelf.
#
# The fixtures (compiled ELF binaries built from tests/*.c by the upstream
# autotools setup) are produced once by rust-patchelf-fixtures and then
# reused by every per-test check.
#
# Run with: nix build .#checks.x86_64-linux.rust-patchelf-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-patchelf-test-set-rpath
{
  pkgs,
  name,
}: let
  fixtures = pkgs.callPackage ./fixtures.nix {};
in
  pkgs.runCommand "rust-patchelf-test-${name}" {
    nativeBuildInputs = [
      pkgs.rust-patchelf-dev
      pkgs.coreutils
      pkgs.binutils
      pkgs.diffutils
      pkgs.bash
      pkgs.gnused
      pkgs.gnugrep
    ];
  } ''
    set -e

    mkdir -p src tests
    cp -rL ${fixtures}/tests/. tests/
    chmod -R u+w tests

    # Upstream test scripts call ../src/patchelf
    ln -s ${pkgs.rust-patchelf-dev}/bin/patchelf src/patchelf

    cd tests

    # Upstream tests reference $srcdir for data files (no-rpath-prebuild,
    # invalid-elf, endianness, contiguous-note-sections.ld, …).
    export srcdir=.

    export PATCHELF_DEBUG=1
    export STRIP=${pkgs.binutils}/bin/strip
    export OBJDUMP=${pkgs.binutils}/bin/objdump
    export READELF=${pkgs.binutils}/bin/readelf
    export OBJCOPY=${pkgs.binutils}/bin/objcopy

    echo "Running upstream patchelf test: ${name}"

    if bash ./${name}.sh; then
      touch $out
    else
      exit 1
    fi
  ''
