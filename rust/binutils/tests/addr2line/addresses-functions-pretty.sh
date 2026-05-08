# Test addr2line -afp — addresses + functions + pretty-print combined
$REF -afp -e "$TESTOBJ" 0x0 0x8 > "$TMPDIR/expected" 2>&1 || true
$RUST -afp -e "$TESTOBJ" 0x0 0x8 > "$TMPDIR/actual" 2>&1 || true
compare "addr2line -afp"
