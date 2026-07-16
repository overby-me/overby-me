"$REF"  -o -l </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" -o -l </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-link -o -l (rich daemon)"
