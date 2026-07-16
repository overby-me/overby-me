# C: printf "%f" defaults to 6 fractional digits — extra precision is
# rounded (banker's: ...789 rounds 7→7 at the 6th place via 89→9 carry,
# so we see '0.123457').
"$REF"  --cutoff=0.123456789 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --cutoff=0.123456789 - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --cutoff=0.123456789 (printf %f rounds to 6 dp → '0.123457')"
