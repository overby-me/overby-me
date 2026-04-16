# Run a single test from the official GNU gawk test suite against rust-awk.
#
# Compares rust-awk output against reference gawk output (both running in the
# same sandbox), avoiding false failures from .ok files generated on
# different systems.
#
# Run with: nix build .#checks.x86_64-linux.rust-awk-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-awk-test-substr
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-awk-test-${name}" {
  nativeBuildInputs = [pkgs.rust-awk-dev pkgs.gawk pkgs.coreutils pkgs.diffutils pkgs.gnused pkgs.gnugrep];
  gawkSrc = pkgs.gawk.src;
} ''
  # Extract the test suite
  tar xf $gawkSrc
  GAWK_SRC=$(echo gawk-*)

  cd "$GAWK_SRC/test"

  export TMPDIR="$(mktemp -d)"

  echo "Running gawk test: ${name}"

  # Determine if an input file exists for this test
  INPUT_ARGS=""
  if [ -f "./${name}.in" ]; then
    INPUT_ARGS="< ./${name}.in"
  fi

  # Run with reference gawk
  eval timeout 60 "${pkgs.gawk}/bin/gawk" -f "./${name}.awk" $INPUT_ARGS > "$TMPDIR/expected" 2>&1 || true

  # Run with rust-awk
  eval timeout 60 "${pkgs.rust-awk-dev}/bin/awk" -f "./${name}.awk" $INPUT_ARGS > "$TMPDIR/actual" 2>&1 || true

  # Normalize binary paths so /nix/store/... differences don't cause false failures
  REF_GAWK="${pkgs.gawk}/bin/gawk"
  TEST_AWK="${pkgs.rust-awk-dev}/bin/awk"
  sed -i "s|$REF_GAWK|awk|g" "$TMPDIR/expected"
  sed -i "s|$TEST_AWK|awk|g" "$TMPDIR/actual"

  # Normalize any remaining nix store paths
  sed -i -E 's|/nix/store/[a-z0-9]{32}-[^/]+|NIXPATH|g' "$TMPDIR/expected" "$TMPDIR/actual"

  # Normalize error message prefixes: "gawk:" → "awk:" and strip source locations
  sed -i 's|^gawk: |awk: |g' "$TMPDIR/expected"
  sed -i -E 's|^awk: \./[^:]+:[0-9]+: |awk: |g' "$TMPDIR/expected" "$TMPDIR/actual"

  # Normalize ARGV[0] differences: gawk reports "gawk" (4), we report "awk" (3)
  sed -i 's|ARGV\[0\] is 4|ARGV[0] is 3|g' "$TMPDIR/expected"

  # Compare
  if diff --text "$TMPDIR/actual" "$TMPDIR/expected"; then
    touch $out
  else
    exit 1
  fi
''
