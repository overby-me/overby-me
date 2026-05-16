# Test objdump -d -M intel-mnemonic — mnemonic-only knob keeps AT&T operand syntax
$REF -d -M intel-mnemonic "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -d -M intel-mnemonic "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -d -M intel-mnemonic"
