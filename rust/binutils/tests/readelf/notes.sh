# Test readelf --notes / -n — display notes sections
$REF --notes "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST --notes "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf --notes"
