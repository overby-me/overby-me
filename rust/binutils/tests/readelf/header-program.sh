# Test readelf -hl — file header + program headers
$REF -hl "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -hl "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -hl"
