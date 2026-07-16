# Sign applies to the whole hex-float value.
"$REF"  --cutoff=-0x1.5p0 - - </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --cutoff=-0x1.5p0 - - </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --cutoff=-0x1.5p0 (signed hex float → '-1.312500')"
