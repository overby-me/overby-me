# Test objdump -t (symbol table)
$REF -t "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -t "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -t (symbols)"