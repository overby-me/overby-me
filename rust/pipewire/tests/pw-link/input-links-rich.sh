"$REF"  -i -l </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" -i -l </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-link -i -l (rich daemon)"
