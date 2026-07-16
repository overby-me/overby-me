# Hex float exponent (pN) is optional — `0x1` parses as 1.0.
"$REF"  --cutoff=0x1 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --cutoff=0x1 - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --cutoff=0x1 (hex int → '1.000000')"
