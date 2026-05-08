# Test size --common — common symbol sizes folded into bss
$REF --common "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST --common "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "size --common"
