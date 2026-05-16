# Test nm -A -P — POSIX format with file name prefix
$REF -A -P "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -A -P "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -A -P"
