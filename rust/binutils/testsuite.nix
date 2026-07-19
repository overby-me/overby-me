# Run a single test from the binutils test suite against rust-binutils.
#
# Compares rust-binutils output against reference GNU binutils output.
#
# Run with: nix build .#checks.x86_64-linux.rust-binutils-test-{tool}-{name}
# Example:  nix build .#checks.x86_64-linux.rust-binutils-test-nm-basic
{
  pkgs,
  tool,
  name,
}: let
  # Map test tool names to actual binary names
  binName =
    if tool == "cxxfilt"
    then "c++filt"
    else tool;
in
  pkgs.runCommand "rust-binutils-test-${tool}-${name}" {
    nativeBuildInputs = [
      pkgs.rust-binutils-dev
      pkgs.binutils-unwrapped
      pkgs.coreutils
      pkgs.diffutils
      pkgs.gnused
      pkgs.gcc
      pkgs.nushell
    ];
    binutilsSrc = pkgs.binutils-unwrapped.src;
    testScript = ./tests/${tool}/${name}.nu;
    testHelpers = ./tests/helpers.nu;
  } ''
    # Extract the upstream source
    tar xf $binutilsSrc
    export SRC=$(echo binutils-*)
    export TESTSUITE="$SRC/binutils/testsuite/binutils-all"

    # Assemble the standard test object
    gcc -c "$TESTSUITE/bintest.s" -o bintest.o
    export TESTOBJ="$PWD/bintest.o"

    export TMPDIR="$(mktemp -d)"

    # Set up tool paths
    export REF="${pkgs.binutils-unwrapped}/bin/${binName}"
    export RUST="${pkgs.rust-binutils-dev}/bin/${binName}"

    # Lay the fixture out one directory below helpers.nu, mirroring the
    # repository layout so the fixture's `source ../helpers.nu` resolves.
    cp $testHelpers helpers.nu
    mkdir fixture
    cp $testScript fixture/test.nu

    echo "Running test: ${tool}/${name}"
    nu fixture/test.nu

    touch $out
  ''
