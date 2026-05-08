# Test readelf -h with two input files
cp "$TESTOBJ" "$TMPDIR/test2.o"
$REF -h "$TESTOBJ" "$TMPDIR/test2.o" > "$TMPDIR/expected" 2>&1 || true
$RUST -h "$TESTOBJ" "$TMPDIR/test2.o" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -h multiple files"
