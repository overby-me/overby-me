# Test nm -g (--extern-only) — show only external symbols
$REF -g "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -g "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --extern-only"