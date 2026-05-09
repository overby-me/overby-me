"$REF"  send-command 0 0 0 </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" send-command 0 0 0 </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli send-command 0 0 0 (no daemon)"
