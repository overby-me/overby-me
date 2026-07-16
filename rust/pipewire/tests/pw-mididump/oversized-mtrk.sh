# MTrk size header says 11 bytes but body is actually 12 bytes. C reads
# past the declared size to find EOT; we should match that tolerance.
printf '\x4D\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x00\x60\x4D\x54\x72\x6B\x00\x00\x00\x0B\x00\xFF\x58\x04\x04\x02\x18\x08\x00\xFF\x2F\x00' > "$TMPDIR/timesig.mid"
"$REF" "$TMPDIR/timesig.mid" </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" "$TMPDIR/timesig.mid" </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-mididump MTrk size off-by-one (read past declared size)"
