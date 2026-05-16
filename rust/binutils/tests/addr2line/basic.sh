# Test addr2line on an object with no debug info — should output ??:0
echo "0x0" | $REF -e "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
echo "0x0" | $RUST -e "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "addr2line basic (no debug info)"