# Test nm -S — BSD format showing symbol sizes
$REF -S "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -S "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -S"
