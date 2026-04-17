# Run a single test from the official GNU make test suite against rust-make.
#
# Drives the upstream Perl test runner (`run_make_tests.pl`) with
# `-make` pointing at the rust-make binary. Exit 0 means pass, anything
# else means fail.
#
# Run with:     nix build .#checks.x86_64-linux.rust-make-test-{category}-{name}
# Example:      nix build .#checks.x86_64-linux.rust-make-test-misc-general1
# View failure: nix log   .#checks.x86_64-linux.rust-make-test-{category}-{name}
{
  pkgs,
  category,
  name,
}:
pkgs.runCommand "rust-make-test-${category}-${name}" {
  nativeBuildInputs = [
    pkgs.rust-make-dev
    pkgs.coreutils
    pkgs.diffutils
    pkgs.gnused
    pkgs.gnugrep
    pkgs.perl
    pkgs.bash
  ];
  gnumakeSrc = pkgs.gnumake.src;
} ''
  # Extract the GNU make test suite
  tar xf $gnumakeSrc
  MAKE_SRC=$(echo make-*)

  cd "$MAKE_SRC/tests"

  export TMPDIR="$(mktemp -d)"

  echo "Running make test: ${category}/${name}"

  # The driver loads config-flags.pm from ../tests/ if present. We skip
  # this file (not shipped in the tarball) — the warning is benign.

  # Run the test. The driver returns 0 on pass, non-zero on fail.
  if timeout 120 ${pkgs.perl}/bin/perl run_make_tests.pl \
       -make ${pkgs.rust-make-dev}/bin/make \
       ${category}/${name} > "$TMPDIR/out" 2>&1; then
    touch $out
  else
    echo "--- test output ---"
    cat "$TMPDIR/out"
    # Include any .diff files the driver produced
    if [ -d work ]; then
      echo "--- work/ contents ---"
      find work -name '*.diff' -o -name '*.log' 2>/dev/null | while read f; do
        echo "=== $f ==="
        cat "$f"
      done
    fi
    exit 1
  fi
''
