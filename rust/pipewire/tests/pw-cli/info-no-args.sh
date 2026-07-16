"$REF"  info </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" info </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli info (no args)"
