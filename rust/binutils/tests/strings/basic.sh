# Test strings on a binary file — should find printable strings
# Create a test file with embedded strings
printf 'garbage\x00\x01\x02This is a test string\x00more garbage\x00\x01Another string here\x00' > "$TMPDIR/testfile"
$REF "$TMPDIR/testfile" > "$TMPDIR/expected" 2>&1 || true
$RUST "$TMPDIR/testfile" > "$TMPDIR/actual" 2>&1 || true
compare "strings basic"