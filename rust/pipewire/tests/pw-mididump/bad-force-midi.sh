"$REF" -M invalid </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -M invalid </dev/null > "$TMPDIR/actual" 2>&1 || true
compare "pw-mididump/bad-force-midi"
