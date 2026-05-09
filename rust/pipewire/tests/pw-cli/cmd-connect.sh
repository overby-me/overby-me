"$REF"  connect </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" connect </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-cli connect"
