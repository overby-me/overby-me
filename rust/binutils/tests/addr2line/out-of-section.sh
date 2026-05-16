# Test addr2line with an out-of-section address — GNU prints `??:0`
# (vs. `??:?` for in-section addresses without DWARF info)
$REF -e "$TESTOBJ" 0x10000 999 > "$TMPDIR/expected" 2>&1 || true
$RUST -e "$TESTOBJ" 0x10000 999 > "$TMPDIR/actual" 2>&1 || true
compare "addr2line out-of-section addresses"
