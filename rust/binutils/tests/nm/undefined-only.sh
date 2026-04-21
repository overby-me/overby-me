# Test nm -u (--undefined-only) — show only undefined symbols
$REF -u "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -u "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --undefined-only"