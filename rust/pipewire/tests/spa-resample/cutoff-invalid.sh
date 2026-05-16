# C: strtod("invalid", NULL) → 0.0, then printf "0.000000".
"$REF"  --cutoff=invalid - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --cutoff=invalid - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --cutoff=invalid (strtod fails → '0.000000')"
