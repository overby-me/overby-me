# spa-resample -q (short --quality) requires an argument.
"$REF"  -q - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -q - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample -q (missing quality argument)"
