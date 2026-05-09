# pw-cli send-command with no args prints `Error: "send-command <usage>"` to stderr.
"$REF"  send-command </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" send-command </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli send-command (usage error)"
