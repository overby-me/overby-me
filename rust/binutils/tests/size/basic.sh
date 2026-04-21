# Test size with no arguments (Berkeley format)
$REF "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "size basic (Berkeley format)"