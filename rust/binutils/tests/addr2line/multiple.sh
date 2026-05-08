# Test addr2line with multiple addresses passed at once
$REF -e "$TESTOBJ" 0 4 8 c > "$TMPDIR/expected" 2>&1 || true
$RUST -e "$TESTOBJ" 0 4 8 c > "$TMPDIR/actual" 2>&1 || true
compare "addr2line multiple"
