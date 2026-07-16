"$REF"  -d help </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" -d help </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-cli -d help"
