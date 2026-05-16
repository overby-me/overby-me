# Test readelf -x .text — should print the "section has relocations" NOTE
$REF -x .text "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -x .text "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -x .text (relocations note)"
