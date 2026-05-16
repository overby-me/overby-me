# Test nm -P -t o — POSIX format with octal radix
$REF -P -t o "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -P -t o "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -P -t o"
