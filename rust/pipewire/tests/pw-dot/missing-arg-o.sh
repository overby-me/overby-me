"$REF" -o </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -o </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-dot/missing-arg-o"
