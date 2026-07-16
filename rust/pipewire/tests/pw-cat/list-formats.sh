"$REF" --list-formats > "$TMPDIR/expected" 2>&1
"$RUST" --list-formats > "$TMPDIR/actual" 2>&1
compare "pw-cat/list-formats"
