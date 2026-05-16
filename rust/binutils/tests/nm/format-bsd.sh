# Test nm -B / --format=bsd — explicit BSD format (default)
$REF -B "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -B "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -B"
