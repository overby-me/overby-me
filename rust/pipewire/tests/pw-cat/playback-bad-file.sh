"$REF"  -p /tmp/nonexistent.wav </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -p /tmp/nonexistent.wav </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat -p /tmp/nonexistent.wav (sndfile error)"
