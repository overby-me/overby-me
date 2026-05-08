# Test nm -t x -A — combined hex radix and file-name prefix
$REF -t x -A "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -t x -A "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -t x -A"
