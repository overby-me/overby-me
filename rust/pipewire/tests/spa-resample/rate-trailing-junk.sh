# atoi takes leading digits only — "44100abc" → 44100, no "bad rate".
"$REF"  --rate=44100abc - - </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" --rate=44100abc - - </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "spa-resample --rate=44100abc (atoi reads 44100, not 'bad rate')"
