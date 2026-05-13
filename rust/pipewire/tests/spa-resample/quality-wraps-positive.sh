# 4294967297 = 2^32 + 1; atoi wraps to 1 (positive) → no 'bad quality'.
"$REF"  --quality=4294967297 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" --quality=4294967297 - - </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "spa-resample --quality=4294967297 (wraps to 1, not 'bad quality')"
