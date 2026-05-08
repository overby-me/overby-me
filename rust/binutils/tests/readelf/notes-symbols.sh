# Test readelf -ns — notes + symbols
$REF -ns "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -ns "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -ns"
