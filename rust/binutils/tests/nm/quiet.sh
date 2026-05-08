# Test nm --quiet — accept the flag (terse archive listing; no-op for single ELF)
$REF --quiet "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST --quiet "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --quiet"
