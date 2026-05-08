# Test readelf -dl — both program headers and dynamic section messages emitted
$REF -dl "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -dl "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -dl (no return after no-program-headers)"
