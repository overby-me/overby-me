# spa-resample -f (short --format) requires an argument.
"$REF"  -f - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -f - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample -f (missing format argument)"
