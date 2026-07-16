# SMF with pitch wheel events.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# 0xe0 ll mh (pitch wheel ch 0, value)
# Center: 0x40 0x00 (0x2000 = 8192)
# Up max: 0x7f 0x7f (0x3fff = 16383)
printf '\x4d\x54\x72\x6b\x00\x00\x00\x0c' >> "$TMPDIR/in.mid"
printf '\x00\xe0\x40\x00' >> "$TMPDIR/in.mid"
printf '\x00\xe0\x7f\x7f' >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/pitch-wheel"
