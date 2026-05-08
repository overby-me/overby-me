# Test nm -t x — print symbol values in hex (default)
$REF -t x "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -t x "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -t x (hex radix)"
