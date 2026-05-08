# Test nm -t o — print symbol values in octal
$REF -t o "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -t o "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -t o (octal radix)"
