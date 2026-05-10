touch "$TMPDIR/locked.wav"
chmod 000 "$TMPDIR/locked.wav"
"$REF"  "$TMPDIR/locked.wav" "$TMPDIR/out.wav" </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" "$TMPDIR/locked.wav" "$TMPDIR/out.wav" </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
chmod 644 "$TMPDIR/locked.wav"  # cleanup
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "spa-resample inaccessible-input (Permission denied)"
