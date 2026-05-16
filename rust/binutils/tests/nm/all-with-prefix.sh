# Test nm -aA — debug-syms (no-op) with file-name prefix
$REF -aA "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -aA "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm -aA"
