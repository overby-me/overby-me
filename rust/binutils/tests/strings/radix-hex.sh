# Test strings -t x — print offset in hex
$REF -t x "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -t x "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "strings -t x"
