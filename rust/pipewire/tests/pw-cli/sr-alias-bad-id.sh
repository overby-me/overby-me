# Short alias `sr` for switch-remote — same error message.
"$REF"  "sr 99" </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" "sr 99" </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli sr 99 (alias, same Remote N does not exist error)"
