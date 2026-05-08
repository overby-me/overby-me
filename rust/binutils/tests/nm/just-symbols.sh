# Test nm -j / --just-symbols — print only symbol names
$REF -j "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -j "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --just-symbols"
