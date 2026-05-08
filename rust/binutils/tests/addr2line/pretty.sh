# Test addr2line -p — pretty-print "function at file:line" instead of newline-separated
$REF -p -e "$TESTOBJ" 0x0 > "$TMPDIR/expected" 2>&1 || true
$RUST -p -e "$TESTOBJ" 0x0 > "$TMPDIR/actual" 2>&1 || true
compare "addr2line -p (pretty)"
