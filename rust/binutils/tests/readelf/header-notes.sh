# Test readelf -nh — combined notes + file header
$REF -nh "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -nh "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -nh"
