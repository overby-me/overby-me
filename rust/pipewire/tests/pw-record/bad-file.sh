"$REF"  /tmp/nonexistent.wav </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" /tmp/nonexistent.wav </dev/null > "$TMPDIR/actual"   2>&1 || true
compare "pw-record /tmp/nonexistent.wav (sndfile error)"
