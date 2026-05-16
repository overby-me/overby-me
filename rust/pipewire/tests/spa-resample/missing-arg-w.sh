# spa-resample -w (short --window) requires an argument.
"$REF"  -w - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -w - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample -w (missing window argument)"
