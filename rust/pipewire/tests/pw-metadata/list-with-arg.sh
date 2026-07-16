"$REF"  --list=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --list=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-metadata --list=foo (no-arg flag)"
