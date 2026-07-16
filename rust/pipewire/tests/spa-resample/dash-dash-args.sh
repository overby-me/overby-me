"$REF"  -- /etc /tmp > "$TMPDIR/expected" 2>&1 || true
"$RUST" -- /etc /tmp > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample -- /etc /tmp (-- terminator + dirs)"
