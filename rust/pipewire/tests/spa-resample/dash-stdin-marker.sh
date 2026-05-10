"$REF"  - /tmp/out.wav </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" - /tmp/out.wav </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "spa-resample - /tmp/out.wav (stdin marker → Format not recognised)"
