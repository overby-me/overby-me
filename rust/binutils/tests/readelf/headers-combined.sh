# Test readelf -hSl — file header + sections + program headers (long form)
$REF -hSl "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -hSl "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -hSl"
