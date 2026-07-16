"$REF" -h > "$TMPDIR/expected" 2>&1
"$RUST" -h > "$TMPDIR/actual" 2>&1
compare "pipewire-aes67/help-short"
