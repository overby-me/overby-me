# Test nm --no-weak — filter out weak symbols
$REF --no-weak "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST --no-weak "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --no-weak"
