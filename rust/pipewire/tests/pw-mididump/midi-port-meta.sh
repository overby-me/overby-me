# Midi Port meta event (0xFF 0x21) — single-byte payload; printer
# formats with zero-padded 3-digit number ("%03d").
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"
printf '\x4d\x54\x72\x6b\x00\x00\x00\x08' >> "$TMPDIR/in.mid"
printf '\x00\xff\x21\x01\x05' >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF"  "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual"   2>&1
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-mididump midi-port-meta (0xFF 0x21 → 'Midi Port: 005')"
