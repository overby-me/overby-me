"$REF"  --batch-mode=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --batch-mode=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-top --batch-mode=foo (no-arg flag)"
