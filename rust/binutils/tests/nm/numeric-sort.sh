# Test nm -n / --numeric-sort — sort symbols by address
$REF -n "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -n "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --numeric-sort"
