# Test readelf -lW — wide-format program headers
$REF -lW "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -lW "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -lW"
