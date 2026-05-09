"$REF"  help </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" help </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-cli help (rich daemon)"
