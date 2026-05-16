# Test strings with two input files
cp "$TESTOBJ" "$TMPDIR/test2.o"
$REF "$TESTOBJ" "$TMPDIR/test2.o" > "$TMPDIR/expected" 2>&1 || true
$RUST "$TESTOBJ" "$TMPDIR/test2.o" > "$TMPDIR/actual" 2>&1 || true
compare "strings multiple files"
