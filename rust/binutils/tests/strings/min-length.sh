# Test strings -n (minimum length)
printf 'ab\x00abcd\x00abcdefgh\x00' > "$TMPDIR/testfile"
$REF -n 5 "$TMPDIR/testfile" > "$TMPDIR/expected" 2>&1 || true
$RUST -n 5 "$TMPDIR/testfile" > "$TMPDIR/actual" 2>&1 || true
compare "strings -n 5 (minimum length)"