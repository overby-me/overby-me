"$REF"  -hX </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -hX </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-json-dump -hX (-h short-circuits to help, X never seen)"
