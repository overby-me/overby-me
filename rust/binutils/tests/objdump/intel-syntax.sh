# Test objdump -d -M intel — Intel-syntax disassembly with size hints
$REF -d -M intel "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -d -M intel "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -d -M intel"
