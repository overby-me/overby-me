"$REF"  --raw=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --raw=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat/raw-with-arg"
