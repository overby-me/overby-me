echo "not audio" > "$TMPDIR/notaudio.wav"
"$REF"  "$TMPDIR/notaudio.wav" /tmp/out.wav </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" "$TMPDIR/notaudio.wav" /tmp/out.wav </dev/null > "$TMPDIR/actual"   2>&1 || true
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "spa-resample existing-bad-format (Format not recognised)"
