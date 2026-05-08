# Test nm -ng — bundled short opts: numeric sort + extern only
$REF -ng "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -ng "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -ng"
