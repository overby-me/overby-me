# C printf %f respects NaN signbit: '-nan' for a NaN with sign bit set.
# Rust's Display always prints 'NaN' regardless of sign, so we test
# both branches of our hand-formatter.
"$REF"  --cutoff=-nan - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --cutoff=-nan - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --cutoff=-nan (printf %f → '-nan' with signbit)"
