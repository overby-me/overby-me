# Test readelf --notes -S — combined notes + section headers
$REF --notes -S "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST --notes -S "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf --notes -S"
