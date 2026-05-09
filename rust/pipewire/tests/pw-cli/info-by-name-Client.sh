# Look up first global of type `Client` by substring match.
"$REF"  info Client </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" info Client </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli info Client (by-name)"
