# Test readelf -c on a non-archive — should print the "not an archive" error
$REF -c "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -c "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -c on non-archive"
