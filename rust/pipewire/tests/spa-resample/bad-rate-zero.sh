"$REF"  --rate=0 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" --rate=0 - - </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "spa-resample --rate=0 (atoi gives 0 → 'bad rate 0')"
