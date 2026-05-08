# Test strings -t o — print offset in octal
$REF -t o "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -t o "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "strings -t o"
