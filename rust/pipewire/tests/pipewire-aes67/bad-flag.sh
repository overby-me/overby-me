"$REF" --bad-flag </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --bad-flag </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pipewire-aes67/bad-flag"
