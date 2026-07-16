"$REF"  --quality > "$TMPDIR/expected" 2>&1 || true
"$RUST" --quality > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --quality (long form requires arg)"
