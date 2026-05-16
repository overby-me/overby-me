# Test objdump -d -j SECTION — disassemble only the named section
$REF -d -j .data "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -d -j .data "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -d -j .data"
