# Test readelf -gW — wide format section groups
$REF -gW "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -gW "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -gW"
