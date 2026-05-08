# Test addr2line -f — function-name resolution via symbol-table fallback
$REF -f -e "$TESTOBJ" 0x0 0x4 0x8 0xc > "$TMPDIR/expected" 2>&1 || true
$RUST -f -e "$TESTOBJ" 0x0 0x4 0x8 0xc > "$TMPDIR/actual" 2>&1 || true
compare "addr2line -f"
