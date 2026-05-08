# Test readelf -x .data — hex dump of .data section
$REF -x .data "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -x .data "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -x .data"
