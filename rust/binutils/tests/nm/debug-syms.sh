# Test nm -a / --debug-syms — include debug symbols (no-op for files without)
$REF -a "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -a "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -a"
