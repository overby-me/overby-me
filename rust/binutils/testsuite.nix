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
    ];
    binutilsSrc = pkgs.binutils-unwrapped.src;
    testScript = ./tests/${tool}/${name}.sh;
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

    # Helper: compare outputs with normalization
    compare() {
      local ref_out="$TMPDIR/expected"
      local rust_out="$TMPDIR/actual"

      # Normalize nix store paths
      sed -i -E 's|/nix/store/[a-z0-9]{32}-[^/[:space:]]+/bin/[a-z+]+|TOOL|g' "$ref_out" "$rust_out"
      sed -i -E 's|/nix/store/[a-z0-9]{32}-[^/[:space:]]+|NIXPATH|g' "$ref_out" "$rust_out"

      # Strip trailing whitespace
      sed -i 's/[[:space:]]*$//' "$ref_out" "$rust_out"

      if diff --text "$rust_out" "$ref_out"; then
        echo "PASS: $1"
      else
        echo "FAIL: $1"
        echo "--- expected (GNU ${tool}) ---"
        cat "$ref_out"
        echo "--- actual (rust-binutils ${tool}) ---"
        cat "$rust_out"
        exit 1
      fi
    }

    echo "Running test: ${tool}/${name}"
    source $testScript

    touch $out
  ''
