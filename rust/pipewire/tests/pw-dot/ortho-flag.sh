# C: case '9' prints "orthogonal edges enabled" to stderr.
"$REF"  --90 </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --90 </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-dot --90 (prints 'orthogonal edges enabled' to stderr)"
