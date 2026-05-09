"$REF"  /tmp/nonexistent /tmp/out.wav </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" /tmp/nonexistent /tmp/out.wav </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample two-positional fail (sndfile error)"
