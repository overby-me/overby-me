# Test nm -gP — extern-only with POSIX format
$REF -gP "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -gP "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -gP"
