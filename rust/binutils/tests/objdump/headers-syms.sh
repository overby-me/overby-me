# Test objdump -ht — section-headers immediately followed by SYMBOL TABLE (no blank between)
$REF -ht "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -ht "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -ht"
