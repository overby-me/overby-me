# Test size -x — Berkeley format with hex radix for individual columns
$REF -x "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -x "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "size -x (hex radix)"
