# Test readelf -s (symbol table)
$REF -s "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -s "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -s (symbols)"