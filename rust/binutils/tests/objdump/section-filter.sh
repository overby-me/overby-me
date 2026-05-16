# Test objdump -j .text -d — disassemble only the .text section
$REF -j .text -d "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -j .text -d "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -j .text -d"
