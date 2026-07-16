# strtod handles sign correctly; printf %f preserves it.
"$REF"  --cutoff=-1.5 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --cutoff=-1.5 - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --cutoff=-1.5 (signed decimal → '-1.500000')"
