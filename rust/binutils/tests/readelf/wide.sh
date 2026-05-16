# Test readelf -S -W — wide format section headers
$REF -S -W "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -S -W "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -S -W"
