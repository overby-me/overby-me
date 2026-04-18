# Run a single test from the upstream Meson test suite against rust-meson.
#
# Each test case is a directory under "test cases/common/" in the meson source.
# Tests are self-validating via assert() in meson.build — failure = non-zero exit.
# Tests that print MESON_SKIP_TEST are treated as skipped (pass).
#
# Run with: nix build .#checks.x86_64-linux.rust-meson-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-meson-test-34-logic-ops
{
  pkgs,
  name,
  testDir,
}:
pkgs.runCommand "rust-meson-test-${name}" {
  nativeBuildInputs = [
    pkgs.rust-meson
    pkgs.ninja
    pkgs.gcc
    pkgs.pkg-config
    pkgs.coreutils
    pkgs.python3
  ];
  mesonSrc = pkgs.meson.src;
} ''
  # Extract the meson source to get test cases
  cp -rL $mesonSrc meson-src
  chmod -R u+w meson-src

  # Copy the test case to a writable working directory
  TEST_CASE="meson-src/test cases/common/${testDir}"
  if [ ! -d "$TEST_CASE" ]; then
    echo "Test case directory not found: $TEST_CASE"
    exit 1
  fi

  mkdir -p workdir
  cp -rL "$TEST_CASE"/* workdir/ 2>/dev/null || true
  cp -rL "$TEST_CASE"/.[!.]* workdir/ 2>/dev/null || true

  # Some tests have subprojects in the meson source tree
  if [ -d "$TEST_CASE/subprojects" ]; then
    cp -rL "$TEST_CASE/subprojects" workdir/subprojects
  fi

  cd workdir

  echo "Running meson test: ${name} (${testDir})"

  # Run meson setup, capturing output
  set +e
  output=$(meson setup builddir 2>&1)
  rc=$?
  set -e

  # Check for MESON_SKIP_TEST — these are legitimate skips (pass)
  if echo "$output" | grep -q "MESON_SKIP_TEST"; then
    echo "SKIP: $output"
    touch $out
    exit 0
  fi

  if [ $rc -ne 0 ]; then
    echo "FAIL: meson setup exited with code $rc"
    echo "$output"
    exit 1
  fi

  echo "PASS: meson setup succeeded"
  echo "$output"
  touch $out
''
