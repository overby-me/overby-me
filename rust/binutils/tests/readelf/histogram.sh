# Test readelf --histogram — bucket histogram for hash sections
$REF --histogram "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST --histogram "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf --histogram"
