"$REF"  --rate=-5 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" --rate=-5 - - </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "spa-resample --rate=-5 (atoi -5 ≤ 0 → 'bad rate -5')"
