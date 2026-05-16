# C: strtod accepts hex floats `0x<mantissa>p<binary exponent>`.
# 0x1.5p0 = (1 + 5/16) * 2^0 = 1.3125 → printf %f = '1.312500'.
"$REF"  --cutoff=0x1.5p0 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --cutoff=0x1.5p0 - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --cutoff=0x1.5p0 (hex float → '1.312500')"
