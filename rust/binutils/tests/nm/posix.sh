# Test nm -P / --portability — POSIX format (name type value size)
$REF -P "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -P "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --portability"
