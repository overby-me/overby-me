# Test readelf -SAW — sections + arch-specific + wide
$REF -SAW "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -SAW "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -SAW"
