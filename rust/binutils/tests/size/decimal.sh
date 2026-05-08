# Test size -d — Berkeley format with decimal radix (default already)
$REF -d "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -d "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "size -d (decimal radix)"
