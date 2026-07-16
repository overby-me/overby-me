"$REF"  -i -I -v </dev/null > "$TMPDIR/expected" 2>&1
"$RUST" -i -I -v </dev/null > "$TMPDIR/actual"   2>&1
compare "pw-link -i -I -v (rich daemon)"
