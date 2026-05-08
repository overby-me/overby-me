# Test objdump --start-address — synthetic label uses <sym+0xoffset> form
$REF -d --start-address=0x4 "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -d --start-address=0x4 "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump --start-address=0x4"
