# Run a single test from the official GNU patch test suite against rust-patch.
#
# The GNU patch tests are shell scripts that source `test-lib.sh` and invoke
# the patch binary via `$PATCH` or `$abs_top_builddir/src/patch`. We extract
# the test suite from the gnupatch source, point `$PATCH` at our rust-patch
# binary, and run each test script in the framework's environment.
#
# Run with: nix build .#checks.x86_64-linux.rust-patch-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-patch-test-bad-usage
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-patch-test-${name}" {
  nativeBuildInputs = [
    pkgs.rust-patch-dev
    pkgs.coreutils
    pkgs.diffutils
    pkgs.gnused
    pkgs.gnugrep
    pkgs.gawk
    pkgs.bash
    pkgs.ed
  ];
  patchSrc = pkgs.gnupatch.src;
} ''
  # Extract the test suite
  tar xf $patchSrc
  PATCH_SRC=$(echo patch-*)

  cd "$PATCH_SRC/tests"

  # Test-lib.sh looks for $srcdir and $abs_top_builddir. tests use_local_patch
  # falls back to $abs_top_builddir/src/patch if $PATCH is unset.
  export srcdir="."
  export abs_top_builddir="$(cd .. && pwd)"
  mkdir -p "$abs_top_builddir/src"
  ln -s ${pkgs.rust-patch-dev}/bin/patch "$abs_top_builddir/src/patch"
  export PATCH="$abs_top_builddir/src/patch"

  export TMPDIR="$(mktemp -d)"

  echo "Running patch test: ${name}"

  # Exit codes: 0 = pass, 77 = skip (also pass), anything else = fail.
  set +e
  timeout 60 bash "./${name}"
  rc=$?
  set -e

  if [ "$rc" -eq 124 ]; then
    echo "TIMEOUT after 60s"
  fi

  if [ "$rc" -eq 0 ] || [ "$rc" -eq 77 ]; then
    echo "PASS (exit code $rc)"
    touch $out
  else
    echo "FAIL (exit code $rc)"
    exit 1
  fi
''
