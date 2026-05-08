# Test addr2line -fp — pretty-print combines function and file:line on one line
$REF -fp -e "$TESTOBJ" 0x0 0x8 > "$TMPDIR/expected" 2>&1 || true
$RUST -fp -e "$TESTOBJ" 0x0 0x8 > "$TMPDIR/actual" 2>&1 || true
compare "addr2line -fp"
