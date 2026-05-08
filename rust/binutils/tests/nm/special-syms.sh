# Test nm --special-syms — accept the flag (no synthetic symbols produced)
$REF --special-syms "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST --special-syms "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --special-syms"
