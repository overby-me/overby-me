# spa-resample -u (short --cutoff) requires an argument.
"$REF"  -u - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -u - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample -u (missing cutoff argument)"
