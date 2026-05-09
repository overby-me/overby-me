"$REF" -M </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -M </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "pw-mididump/missing-M-arg"
