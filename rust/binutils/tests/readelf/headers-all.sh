# Test readelf -e — file header + section headers + program headers
$REF -e "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -e "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -e (all headers)"
