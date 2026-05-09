"$REF"  --all=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --all=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-dot --all=foo (no-arg flag)"
