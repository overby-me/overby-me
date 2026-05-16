# Test objdump -d (disassemble)
$REF -d "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -d "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -d (disassemble)"