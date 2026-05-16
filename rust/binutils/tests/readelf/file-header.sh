# Test readelf -h (file header)
$REF -h "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -h "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -h (file header)"