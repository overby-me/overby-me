# Test objdump -h (section headers)
$REF -h "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -h "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -h (section headers)"