"$REF"  connect new-name </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" connect new-name </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cli connect <bad-name>"
