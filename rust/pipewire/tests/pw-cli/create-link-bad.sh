"$REF"  create-link 1 2 3 4 </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" create-link 1 2 3 4 </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli create-link 1 2 3 4 (no daemon)"
