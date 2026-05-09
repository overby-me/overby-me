"$REF"  create-link </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" create-link </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli create-link (usage error)"
