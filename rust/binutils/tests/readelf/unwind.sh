# Test readelf -u — unwind information dump (no processor-specific decoder)
$REF -u "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -u "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -u"
