"$REF"  -q-1 - - > "$TMPDIR/expected" 2>&1 || true
"$RUST" -q-1 - - > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample -q-1 (negative quality → 'bad quality -1')"
