# Test strings -t d — print offset in decimal
$REF -t d "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -t d "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "strings -t d"
