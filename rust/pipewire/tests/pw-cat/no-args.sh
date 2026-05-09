# pw-cat with no args prints an error then the help block.
"$REF" </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat/no-args"
