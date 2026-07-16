"$REF"  -M=ump </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -M=ump </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mididump -M=ump (literal '=ump' value)"
