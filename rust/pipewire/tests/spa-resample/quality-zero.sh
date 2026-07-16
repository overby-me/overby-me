# C: ret = atoi("0") = 0, NOT < 0 → no "bad quality". Proceeds to file open.
"$REF"  --quality=0 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" --quality=0 - - </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "spa-resample --quality=0 (atoi 0 is not <0 → file open error, not 'bad quality')"
