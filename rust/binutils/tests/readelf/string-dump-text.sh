# Test readelf -p .text — should print the "section has relocations" NOTE
$REF -p .text "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -p .text "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -p .text (relocations note)"
