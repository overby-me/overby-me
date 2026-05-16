# Test nm -t d — print symbol values in decimal
$REF -t d "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -t d "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -t d (decimal radix)"
