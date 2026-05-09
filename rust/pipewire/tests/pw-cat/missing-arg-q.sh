"$REF" -q </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -q </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat/missing-arg-q"
