# Test nm -P -t d — POSIX format with decimal radix
$REF -P -t d "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -P -t d "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -P -t d"
