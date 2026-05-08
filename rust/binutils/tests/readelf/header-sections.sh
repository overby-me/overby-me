# Test readelf -hS — file header + section headers
$REF -hS "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -hS "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -hS"
