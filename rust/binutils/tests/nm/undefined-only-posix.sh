# Test nm -uP — undefined-only with POSIX format
$REF -uP "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -uP "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -uP"
