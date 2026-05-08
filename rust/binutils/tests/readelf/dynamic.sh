# Test readelf -d / --dynamic — display dynamic section (or "no dynamic" message for .o)
$REF -d "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -d "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -d (dynamic section)"
