# Test objdump -t -j SECTION — symbol table filtered to one section
$REF -t -j .text "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -t -j .text "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -t -j .text"
