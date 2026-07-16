"$REF" --rate </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --rate </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat/missing-arg-rate"
