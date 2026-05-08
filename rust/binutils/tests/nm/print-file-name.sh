# Test nm -A / --print-file-name — prefix output with file name
$REF -A "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -A "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --print-file-name"
