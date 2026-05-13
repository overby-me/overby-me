# atoi("") = 0, not <0 → not 'bad quality'.
"$REF"  --quality= - - </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" --quality= - - </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "spa-resample --quality= (atoi empty = 0, not <0 → no 'bad quality')"
