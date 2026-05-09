"$REF"  --rate=100 a </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --rate=100 a </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --rate=100 a (inline value)"
