# Look up first global of type `Module` by substring match.
"$REF"  info Module </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" info Module </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli info Module (by-name)"
