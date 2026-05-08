# Test objdump -h -j SECTION — section-headers filtered to one section
$REF -h -j .text "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -h -j .text "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -h -j .text"
