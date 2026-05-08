# Test readelf -nW — Properties on the same line as the description (wide mode)
$REF -nW "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -nW "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -nW (inline properties)"
