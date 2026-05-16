# C: strtod("inf") → INFINITY; printf "%f" of inf → "inf" (no '.000000').
# Rust's default Display for f64::INFINITY prints "inf" too, so this
# matches even with the generic {:.6} formatter.
"$REF"  --cutoff=inf - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --cutoff=inf - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --cutoff=inf (printf %f → 'inf')"
