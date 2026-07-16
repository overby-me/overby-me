"$REF"  --verbose=foo </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" --verbose=foo </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample --verbose=foo (no-arg flag)"
