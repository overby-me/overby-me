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
    pkgs.git
    pkgs.unzip
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

    # Some tests (e.g. 220 fs module) need a valid home directory
    export HOME=/tmp

    # Extract -D options from test.json if it exists
    MESON_OPTS_FILE=$(mktemp)
    TEST_JSON="/build/meson-src/test cases/common/${testDir}/test.json"
    if [ -f "$TEST_JSON" ]; then
      ${pkgs.python3}/bin/python3 -c "
  import json, sys, shlex
  try:
      with open(sys.argv[1]) as f:
          data = json.load(f)
      matrix = data.get('matrix', {})
      options = matrix.get('options', {})
      for key, vals in options.items():
          if vals and isinstance(vals[0], dict) and 'val' in vals[0]:
              val = vals[0]['val']
              if isinstance(val, list):
                  val = ','.join(str(v) for v in val)
              elif isinstance(val, bool):
                  val = str(val).lower()
              print(shlex.quote(f'-D{key}={val}'))
  except Exception:
      pass
  " "$TEST_JSON" > "$MESON_OPTS_FILE" 2>/dev/null || true
    fi

    echo "Running meson test: ${name} (${testDir})"

    # Build meson setup command with extracted options
    MESON_CMD="meson setup builddir"
    while IFS= read -r opt; do
      [ -n "$opt" ] && MESON_CMD="$MESON_CMD $opt"
    done < "$MESON_OPTS_FILE"
    rm -f "$MESON_OPTS_FILE"

    # Auto-detect native file in test directory
    HAVE_NATIVE=0
    for nf in nativefile.ini native.txt cross_file.txt; do
      if [ -f "$nf" ]; then
        MESON_CMD="$MESON_CMD --native-file $nf"
        HAVE_NATIVE=1
        break
      fi
    done
    # Auto-detect cross file in test directory (only if no native file)
    if [ "$HAVE_NATIVE" = "0" ]; then
      for cf in crossfile.ini cross.txt cross_file.ini; do
        if [ -f "$cf" ]; then
          MESON_CMD="$MESON_CMD --cross-file $cf"
          break
        fi
      done
    fi

    # Run meson setup, capturing output
    set +e
    output=$(eval $MESON_CMD 2>&1)
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
