# C: strtod consumes the longest valid float prefix; "1.5xyz" → 1.5.
# Rust's f64::parse rejects the whole string, so we walk back to find
# the longest accepted prefix.
"$REF"  --cutoff=1.5xyz - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --cutoff=1.5xyz - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --cutoff=1.5xyz (strtod prefix → '1.500000')"
