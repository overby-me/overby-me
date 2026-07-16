"$REF"  create-node foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" create-node foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli create-node foo (no daemon)"
