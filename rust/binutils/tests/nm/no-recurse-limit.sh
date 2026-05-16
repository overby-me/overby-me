# Test nm --no-recurse-limit — accept the demangler knob as a no-op
$REF --no-recurse-limit "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST --no-recurse-limit "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm --no-recurse-limit"
