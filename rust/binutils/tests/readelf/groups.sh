# Test readelf -g / --section-groups — display section groups
$REF -g "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -g "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -g (section groups)"
