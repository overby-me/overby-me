# Test size -A -x — sysv format with hex radix
$REF -A -x "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -A -x "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "size -A -x"
