# Test size -A (SysV format)
$REF -A "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -A "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "size -A (SysV format)"