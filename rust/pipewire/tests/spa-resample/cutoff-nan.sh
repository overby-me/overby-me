# C: strtod("nan") → NaN; printf "%f" prints "nan" (lowercase). Rust's
# default Display prints "NaN" (uppercase) so we must hand-format.
"$REF"  --cutoff=nan - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --cutoff=nan - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --cutoff=nan (printf %f → 'nan' lowercase)"
