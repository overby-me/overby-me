# Test objdump -dM intel — bundled short opt with -M (takes argument)
$REF -dM intel "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -dM intel "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -dM intel"
