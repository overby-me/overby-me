# spa-resample -t (short --taps) requires an argument.
"$REF"  -t - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -t - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample -t (missing taps argument)"
