# Test size -A -o — sysv format with octal radix
$REF -A -o "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -A -o "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "size -A -o"
