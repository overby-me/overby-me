# Run upstream GNU binutils DejaGnu tests against rust-binutils.
#
# Runs the real `.exp` test files from the binutils source tree using
# DejaGnu's `runtest` harness, with all tool paths pointed at
# rust-binutils symlinks.
#
# Usage:
#   Single .exp file:
#     nix build .#checks.x86_64-linux.rust-binutils-dejagnu-size
#
#   All upstream tests:
#     nix build .#checks.x86_64-linux.rust-binutils-dejagnu-all
{
  pkgs,
  # Which .exp file(s) to run.  Pass a single name like "size.exp" or
  # "ALL" to run every .exp file in binutils-all/.
  expFile ? "ALL",
  # Minimum number of expected passes required for the check to succeed.
  # Set to 0 to make the check informational (always succeeds).
  minPass ? 0,
  # Maximum number of unexpected failures allowed.  Set to a high number
  # to make the check informational.
  maxFail ? 999999,
}: let
  name =
    if expFile == "ALL"
    then "rust-binutils-dejagnu-all"
    else "rust-binutils-dejagnu-${builtins.replaceStrings [".exp"] [""] expFile}";
in
  pkgs.runCommand name {
    nativeBuildInputs = [
      pkgs.rust-binutils-dev
      pkgs.dejagnu
      pkgs.expect
      pkgs.tcl
      pkgs.gcc
      pkgs.binutils-unwrapped # for `as` and `ld` (used by binutils_assemble)
      pkgs.coreutils
      pkgs.gnused
      pkgs.gnugrep
      pkgs.gawk
      pkgs.diffutils
    ];
    binutilsSrc = pkgs.binutils-unwrapped.src;
  } ''
        # ── Extract upstream source ──────────────────────────────────────────
        tar xf $binutilsSrc
        SRC=$(echo binutils-*)
        TESTSUITE="$PWD/$SRC/binutils/testsuite"

        # ── Set up working directory ─────────────────────────────────────────
        # DejaGnu expects to be run from a directory that contains site.exp
        # and a tmpdir/ for assembler output.
        # The ar.exp test "replacing non-deterministic member" explicitly expects
        # SOURCE_DATE_EPOCH to NOT be set (Nix's build env sets it). Unset it so the
        # tests for both deterministic and non-deterministic ar archives can distinguish.
        unset SOURCE_DATE_EPOCH

        WORKDIR=$(mktemp -d)
        cd "$WORKDIR"
        mkdir -p tmpdir

        # ── Locate rust-binutils ─────────────────────────────────────────────
        RUST_BIN="${pkgs.rust-binutils-dev}/bin"

        # ── Create site.exp ──────────────────────────────────────────────────
        cat > site.exp << SITE_EOF
    set host_triplet "x86_64-unknown-linux-gnu"
    set target_triplet "x86_64-unknown-linux-gnu"
    set target_alias "x86_64-unknown-linux-gnu"
    set build_triplet "x86_64-unknown-linux-gnu"
    set srcdir "$TESTSUITE"
    set objdir "$WORKDIR"
    set tool binutils
    set experimental false

    set NM "$RUST_BIN/nm"
    set NMFLAGS ""
    set AR "$RUST_BIN/ar"
    set SIZE "$RUST_BIN/size"
    set SIZEFLAGS ""
    set OBJDUMP "$RUST_BIN/objdump"
    set OBJDUMPFLAGS ""
    set OBJCOPY "$RUST_BIN/objcopy"
    set OBJCOPYFLAGS ""
    set STRINGS "$RUST_BIN/strings"
    set STRINGSFLAGS ""
    set STRIP "$RUST_BIN/strip"
    set STRIPFLAGS ""
    set READELF "$RUST_BIN/readelf"
    set READELFFLAGS ""
    set ADDR2LINE "$RUST_BIN/addr2line"
    set ADDR2LINEFLAGS ""
    set CXXFILT "$RUST_BIN/c++filt"
    set CXXFILTFLAGS ""

    set AS "${pkgs.binutils-unwrapped}/bin/as"
    set ASFLAGS ""
    set AS_FOR_TARGET "${pkgs.binutils-unwrapped}/bin/as"
    set LD "${pkgs.binutils-unwrapped}/bin/ld"
    set LDFLAGS ""
    set CC "${pkgs.gcc}/bin/cc"
    set CC_FOR_TARGET "${pkgs.gcc}/bin/cc"
    set CFLAGS ""
    set CFLAGS_FOR_TARGET ""
    set gcc_gas_flag "-B${pkgs.binutils-unwrapped}/bin/"
    set dlltool_gas_flag ""
    SITE_EOF

        # ── Determine which .exp files to run ────────────────────────────────
        EXP_FILES=""
        if [ "${expFile}" = "ALL" ]; then
          for f in "$TESTSUITE"/binutils-all/*.exp; do
            base=$(basename "$f")
            # Skip tests for tools we don't implement or that need special setup
            case "$base" in
              dlltool.exp|elfedit.exp|debuginfod.exp)
                continue ;;
            esac
            EXP_FILES="$EXP_FILES $base"
          done
        else
          EXP_FILES="${expFile}"
        fi

        echo "Running DejaGnu tests: $EXP_FILES"
        echo "Tool binaries from: $RUST_BIN"
        echo ""

        # ── Run the tests ────────────────────────────────────────────────────
        # -a: show all results (including passes)
        # runtest returns non-zero if any test fails; we parse the summary
        # ourselves so we ignore its exit code.
        runtest \
          -a \
          --tool binutils \
          --srcdir "$TESTSUITE" \
          $EXP_FILES \
          > test-output.log 2>&1 || true

        cat test-output.log

        echo ""
        echo "════════════════════════════════════════════════════════════"

        # ── Parse results ────────────────────────────────────────────────────
        # Use awk for robust counting (grep -c can misbehave with empty matches)
        PASSES=$(awk '/^PASS:/{n++} END{print n+0}' test-output.log)
        FAILURES=$(awk '/^FAIL:/{n++} END{print n+0}' test-output.log)
        XFAILS=$(awk '/^XFAIL:/{n++} END{print n+0}' test-output.log)
        UNRESOLVED=$(awk '/^UNRESOLVED:/{n++} END{print n+0}' test-output.log)
        UNTESTED=$(awk '/^UNTESTED:/{n++} END{print n+0}' test-output.log)

        echo "Results: $PASSES passed, $FAILURES failed, $XFAILS expected-fail, $UNRESOLVED unresolved, $UNTESTED untested"
        echo ""

        if [ "$FAILURES" -gt 0 ]; then
          echo "Failed tests:"
          grep '^FAIL:' test-output.log | sed 's/^/  /' || true
          echo ""
        fi

        # ── Save detailed results ────────────────────────────────────────────
        mkdir -p $out
        cp test-output.log $out/
        cp -f binutils.log binutils.sum $out/ 2>/dev/null || true
        grep '^PASS:\|^FAIL:\|^XFAIL:\|^UNRESOLVED:\|^UNTESTED:' test-output.log \
          > $out/summary.log 2>/dev/null || true

        cat > $out/results.txt << RESULTS_EOF
    passes=$PASSES
    failures=$FAILURES
    xfails=$XFAILS
    unresolved=$UNRESOLVED
    untested=$UNTESTED
    RESULTS_EOF

        # ── Check thresholds ─────────────────────────────────────────────────
        MINPASS=${toString minPass}
        MAXFAIL=${toString maxFail}

        if [ "$PASSES" -lt "$MINPASS" ]; then
          echo "THRESHOLD FAILURE: only $PASSES passes (minimum: $MINPASS)"
          exit 1
        fi

        if [ "$FAILURES" -gt "$MAXFAIL" ]; then
          echo "THRESHOLD FAILURE: $FAILURES failures (maximum: $MAXFAIL)"
          exit 1
        fi

        echo "Thresholds OK: passes=$PASSES >= $MINPASS, failures=$FAILURES <= $MAXFAIL"
  ''
