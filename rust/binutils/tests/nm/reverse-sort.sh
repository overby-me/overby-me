# Test nm -r / --reverse-sort — reverse the sort order
$REF -r "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -r "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --reverse-sort"
