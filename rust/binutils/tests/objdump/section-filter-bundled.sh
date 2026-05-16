# Test objdump -dj.text — bundled short opt with -j (takes argument, no separator)
$REF -dj.text "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -dj.text "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "objdump -dj.text"
