# Test nm -U / --defined-only — show only symbols with addresses
$REF --defined-only "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST --defined-only "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --defined-only"
