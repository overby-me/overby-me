"$REF"  -o -v </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" -o -v </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-link -o -v (rich daemon)"
