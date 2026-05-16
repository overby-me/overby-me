# Test strings on an object file
$REF "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "strings on object file"