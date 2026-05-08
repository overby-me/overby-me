# Test objdump -dz — disassemble zero bytes (no `\t...` collapse)
$REF -dz "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -dz "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -dz"
