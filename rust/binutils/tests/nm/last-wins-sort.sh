# Test nm bundled sort flags — last-wins between -n (numeric) and -p (no-sort)
$REF -np "$TESTOBJ" > "$TMPDIR/expected" 2>&1 || true
$RUST -np "$TESTOBJ" > "$TMPDIR/actual" 2>&1 || true
compare "nm bundled flags (last-wins direction 1)"

ARGS=$(printf -- "-%s" p)$(printf -- "%s" n)
$REF "$ARGS" "$TESTOBJ" >> "$TMPDIR/expected" 2>&1 || true
$RUST "$ARGS" "$TESTOBJ" >> "$TMPDIR/actual" 2>&1 || true
compare "nm bundled flags (last-wins both)"
