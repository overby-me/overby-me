# 4294967297 = 2^32 + 1; atoi wraps to 1 (positive) → no "bad rate".
# Proceeds to filename check / sndfile open.
"$REF"  --rate=4294967297 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" --rate=4294967297 - - </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "spa-resample --rate=4294967297 (wraps to 1 via atoi, not 'bad rate')"
