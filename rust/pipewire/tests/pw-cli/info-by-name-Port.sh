"$REF"  info Port </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" info Port </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli info Port (by-name lookup, rich daemon)"
