"$REF"  --rate > "$TMPDIR/expected" 2>&1 || true
"$RUST" --rate > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --rate (long form requires arg)"
