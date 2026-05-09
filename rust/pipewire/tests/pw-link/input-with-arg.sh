"$REF"  --input=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --input=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-link --input=foo (no-arg flag rejects value)"
