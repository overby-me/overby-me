# Test nm -j -A — `-j` should drop the file-name prefix from `-A`
$REF -j -A "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -j -A "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -j -A"
