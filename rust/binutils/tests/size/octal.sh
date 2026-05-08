# Test size -o — Berkeley format with octal radix for individual columns
$REF -o "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -o "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "size -o (octal radix)"
