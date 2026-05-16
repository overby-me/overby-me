# Test readelf -r / --relocs — display relocations
$REF -r "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -r "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -r"
