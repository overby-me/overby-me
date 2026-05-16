# Test addr2line -i / --inlines — show inlined functions
$REF -i -e "$TESTOBJ" 0x0 > "$TMPDIR/expected" 2>&1 || true
$RUST -i -e "$TESTOBJ" 0x0 > "$TMPDIR/actual" 2>&1 || true
compare "addr2line -i"
