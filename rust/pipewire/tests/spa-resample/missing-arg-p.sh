# spa-resample -p (short --param) requires an argument.
"$REF"  -p - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -p - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample -p (missing param argument)"
