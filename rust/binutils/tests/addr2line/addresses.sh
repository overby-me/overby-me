# Test addr2line -a — show address before each entry
$REF -a -e "$TESTOBJ" 0x0 > "$TMPDIR/expected" 2>&1 || true
$RUST -a -e "$TESTOBJ" 0x0 > "$TMPDIR/actual" 2>&1 || true
compare "addr2line -a"
