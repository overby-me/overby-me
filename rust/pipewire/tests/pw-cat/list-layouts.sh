"$REF" --list-layouts > "$TMPDIR/expected" 2>&1
"$RUST" --list-layouts > "$TMPDIR/actual" 2>&1
compare "pw-cat/list-layouts"
