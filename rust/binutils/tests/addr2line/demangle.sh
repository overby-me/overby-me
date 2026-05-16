# Test addr2line -C — demangle function names
$REF -C -e "$TESTOBJ" 0x0 > "$TMPDIR/expected" 2>&1 || true
$RUST -C -e "$TESTOBJ" 0x0 > "$TMPDIR/actual" 2>&1 || true
compare "addr2line -C"
