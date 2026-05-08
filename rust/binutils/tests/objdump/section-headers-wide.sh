# Test objdump -hw — wide section headers (single-line per section, with Flags)
$REF -hw "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -hw "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -hw"
