"$REF"  create-device foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" create-device foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli create-device foo (no daemon)"
