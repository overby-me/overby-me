# Run a single test from the official GNU tar test suite against rust-tar.
#
# The GNU tar tests are Autotest (.at) files that autom4te compiles into a
# single `tests/testsuite` shell script. We rely on the prebuilt
# `gnutar-test-harness` derivation for the generated script + helper
# binaries (`genfile`, `checkseekhole`, `ckmtime`). Each per-test
# derivation looks up the test's numeric id by matching the .at filename
# in `testsuite -l`, then runs just that id with `TAR` pointed at
# rust-tar. We can't use `-k NAME` because Autotest keywords are often
# shared across multiple tests (e.g. `append` is a keyword on every
# append*.at file).
#
# Run with: nix build .#checks.x86_64-linux.rust-tar-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-tar-test-append
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-tar-test-${name}" {
  nativeBuildInputs = [
    pkgs.rust-tar-dev
    pkgs.coreutils
    pkgs.diffutils
    pkgs.gnused
    pkgs.gnugrep
    pkgs.gawk
    pkgs.bash
    pkgs.attr
    pkgs.acl
    pkgs.gzip
    pkgs.bzip2
    pkgs.xz
  ];
  harness = pkgs.gnutar-test-harness;
} ''
  # Prepare a writable copy of the prebuilt harness so the testsuite can
  # write to its tests/ and tmp-* dirs.
  cp -r $harness/tar-1.35 tar-src
  chmod -R u+w tar-src
  cd tar-src

  # Replace the src/tar binary with rust-tar so the testsuite picks it
  # up via atlocal's PATH tweak.
  rm -f src/tar
  ln -sf ${pkgs.rust-tar-dev}/bin/tar src/tar
  export TAR=${pkgs.rust-tar-dev}/bin/tar

  cd tests

  # Resolve the .at filename → numeric test id by scanning
  # `testsuite -l`. The format is lines like "  49: append.at:21   append".
  ./testsuite -l > ts-list.txt 2>&1 || true
  test_id=$(awk -v n='${name}' '
    $1 ~ /^[0-9]+:$/ && $2 ~ "^"n"\\.at:" { gsub(":", "", $1); print $1; exit }
  ' ts-list.txt)
  if [ -z "$test_id" ]; then
    echo "No test id found for ${name}" >&2
    tail -50 ts-list.txt
    exit 1
  fi

  echo "Running tar test: ${name} (id $test_id)"

  set +e
  timeout 600 bash ./testsuite "$test_id" -v 2>&1 | tee testsuite.out
  rc=''${PIPESTATUS[0]}
  set -e

  if [ "$rc" -eq 0 ]; then
    echo "PASS (exit code $rc)"
    touch $out
  else
    echo "FAIL (exit code $rc)"
    for f in testsuite.dir/*/testsuite.log; do
      if [ -f "$f" ]; then
        echo "=== $f ==="
        cat "$f"
      fi
    done
    exit 1
  fi
''
