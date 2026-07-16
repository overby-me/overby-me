"$REF" --list-containers > "$TMPDIR/expected" 2>&1
"$RUST" --list-containers > "$TMPDIR/actual" 2>&1
compare "pw-cat/list-containers"
