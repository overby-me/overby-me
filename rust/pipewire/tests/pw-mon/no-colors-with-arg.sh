"$REF"  --no-colors=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --no-colors=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-mon --no-colors=foo (no-arg flag)"
