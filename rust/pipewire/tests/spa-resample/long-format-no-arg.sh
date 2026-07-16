"$REF"  --format > "$TMPDIR/expected" 2>&1 || true
"$RUST" --format > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --format (long form requires arg)"
