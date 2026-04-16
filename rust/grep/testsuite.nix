# Run a single test from the official GNU grep test suite against rust-grep.
#
# The GNU grep tests are shell scripts that use the gnulib test framework.
# We extract the test suite from the gnugrep source, place our rust-grep
# binary first on PATH (so it's found as "grep"), and run each test script
# in the framework's environment.
#
# Run with: nix build .#checks.x86_64-linux.rust-grep-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-grep-test-backref
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-grep-test-${name}" {
  nativeBuildInputs = [pkgs.rust-grep pkgs.coreutils pkgs.diffutils pkgs.gnused pkgs.gnugrep pkgs.gawk pkgs.bash pkgs.perl];
  grepSrc = pkgs.gnugrep.src;
} ''
  # Extract the test suite
  tar xf $grepSrc
  GREP_SRC=$(echo grep-*)

  cd "$GREP_SRC/tests"

  export TMPDIR="$(mktemp -d)"

  # Make the test framework available
  export srcdir="."
  export abs_top_srcdir="$(pwd)/.."
  export abs_srcdir="$(pwd)"

  # Create a src/ directory with our grep binary (tests use path_prepend_ ../src)
  mkdir -p "../src"
  ln -s ${pkgs.rust-grep}/bin/grep "../src/grep"
  ln -s ${pkgs.rust-grep}/bin/egrep "../src/egrep"
  ln -s ${pkgs.rust-grep}/bin/fgrep "../src/fgrep"

  # Also create a bin directory for direct PATH usage
  mkdir -p "$TMPDIR/bin"
  ln -s ${pkgs.rust-grep}/bin/grep "$TMPDIR/bin/grep"
  ln -s ${pkgs.rust-grep}/bin/egrep "$TMPDIR/bin/egrep"
  ln -s ${pkgs.rust-grep}/bin/fgrep "$TMPDIR/bin/fgrep"

  # Put our grep first on PATH, but keep system tools available
  export PATH="$TMPDIR/bin:${pkgs.coreutils}/bin:${pkgs.diffutils}/bin:${pkgs.gnused}/bin:${pkgs.gawk}/bin:${pkgs.bash}/bin:${pkgs.perl}/bin:/usr/bin:/bin"

  echo "Running grep test: ${name}"

  # Run the test script
  # Exit codes: 0 = pass, 77 = skip (also pass), 1+ = fail
  set +e
  bash "./${name}" 9>&2 > "$TMPDIR/stdout" 2>&1
  rc=$?
  set -e

  cat "$TMPDIR/stdout"

  if [ "$rc" -eq 0 ] || [ "$rc" -eq 77 ]; then
    echo "PASS (exit code $rc)"
    touch $out
  else
    echo "FAIL (exit code $rc)"
    exit 1
  fi
''
