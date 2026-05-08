# Test objdump -f / --file-headers — display file header info
$REF -f "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -f "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -f"
