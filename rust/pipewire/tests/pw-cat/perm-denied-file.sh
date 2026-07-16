# Create a file we can't read
touch "$TMPDIR/locked.wav"
chmod 000 "$TMPDIR/locked.wav"
"$REF"  -p "$TMPDIR/locked.wav" </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" -p "$TMPDIR/locked.wav" </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
chmod 644 "$TMPDIR/locked.wav"  # cleanup
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-cat -p inaccessible-file (sndfile permission-denied)"
