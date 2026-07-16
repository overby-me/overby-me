"$REF" "$TMPDIR/nonexistent.mid" </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" "$TMPDIR/nonexistent.mid" </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-midiplay missing-file (midifile: can't read)"
