# Look up first global of type `Factory` by substring match.
"$REF"  info Factory </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" info Factory </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli info Factory (by-name)"
