# Test readelf -dA — dynamic + arch-specific (both empty for relocatables)
$REF -dA "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -dA "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -dA"
