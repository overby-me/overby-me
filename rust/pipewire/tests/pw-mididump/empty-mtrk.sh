printf '\x4D\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x00\x60\x4D\x54\x72\x6B\x00\x00\x00\x00' > "$TMPDIR/empty.mid"
"$REF" "$TMPDIR/empty.mid" </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" "$TMPDIR/empty.mid" </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-mididump empty MTrk (size=0, no events)"
