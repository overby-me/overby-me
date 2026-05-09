"$REF"  --monitor=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --monitor=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-metadata --monitor=foo (no-arg flag)"
