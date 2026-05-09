"$REF" --list-channel-names > "$TMPDIR/expected" 2>&1
"$RUST" --list-channel-names > "$TMPDIR/actual" 2>&1
compare "pw-cat/list-channel-names"
