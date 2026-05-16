# Test readelf -S (section headers)
$REF -S "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -S "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -S (section headers)"