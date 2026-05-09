"$REF"  --lr=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --lr=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-dot --lr=foo (no-arg flag)"
