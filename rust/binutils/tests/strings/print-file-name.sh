# Test strings -f — prefix each string with the input file name
$REF -f "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -f "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "strings -f"
