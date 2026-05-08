# Test readelf -A / --arch-specific — display architecture-specific info (none for x86)
$REF -A "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -A "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -A"
