echo "not audio" > "$TMPDIR/notaudio.wav"
"$REF"  -p "$TMPDIR/notaudio.wav" </dev/null > "$TMPDIR/expected" 2>&1 || true
"$RUST" -p "$TMPDIR/notaudio.wav" </dev/null > "$TMPDIR/actual"   2>&1 || true
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-cat -p existing-bad-format (Format not recognised)"
