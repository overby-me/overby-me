# Test objdump -dt — symbol table followed by disassembly with correct blank-line spacing
$REF -dt "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -dt "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -dt"
