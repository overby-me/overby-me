# Test readelf -p .comment — produces "section not dumped" warning when missing
$REF -p .comment "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -p .comment "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "readelf -p .comment (missing)"
