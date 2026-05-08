# Test readelf --syms --use-dynamic on file without dynamic symbols
$REF --syms --use-dynamic "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST --syms --use-dynamic "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf --syms --use-dynamic"
