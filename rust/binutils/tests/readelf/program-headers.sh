# Test readelf -l (program headers) on the test object
# Object files typically have no program headers, so output should indicate that
$REF -l "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -l "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -l (program headers)"