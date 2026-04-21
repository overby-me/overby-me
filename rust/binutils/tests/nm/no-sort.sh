# Test nm -p (--no-sort) — display symbols in order encountered
$REF -p "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -p "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --no-sort"