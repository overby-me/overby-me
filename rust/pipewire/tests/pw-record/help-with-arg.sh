"$REF" --help=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --help=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-record/help-with-arg"
