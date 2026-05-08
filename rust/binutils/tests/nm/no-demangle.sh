# Test nm --no-demangle — accept the flag (no-op since we don't demangle by default)
$REF --no-demangle "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST --no-demangle "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --no-demangle"
