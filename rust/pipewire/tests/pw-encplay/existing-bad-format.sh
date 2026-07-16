echo "not media" > "$TMPDIR/notaudio.txt"
"$REF" "$TMPDIR/notaudio.txt" </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" "$TMPDIR/notaudio.txt" </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
sed -i "s|$TMPDIR|TMPDIR|g; s|0x[0-9a-fA-F]*|0xPTR|g; s|Statistics: [0-9]* bytes read|Statistics: BYTES bytes read|g" "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-encplay existing-bad-format (avformat: Invalid data)"
