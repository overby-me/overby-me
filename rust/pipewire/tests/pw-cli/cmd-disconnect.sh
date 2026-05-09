"$REF"  disconnect </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" disconnect </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-cli disconnect"
