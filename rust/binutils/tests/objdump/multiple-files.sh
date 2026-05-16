# Test objdump -f with two input files
cp "$TESTOBJ" "$TMPDIR/test2.o"
$REF -f "$TESTOBJ" "$TMPDIR/test2.o" > "$TMPDIR/expected" 2>&1 || true
$RUST -f "$TESTOBJ" "$TMPDIR/test2.o" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -f multiple files"
