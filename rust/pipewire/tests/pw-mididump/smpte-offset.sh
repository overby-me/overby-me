# SMF with SMPTE offset meta event.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# 0xff 0x54 0x05 hr mn se fr ff (SMPTE offset)
printf '\x4d\x54\x72\x6b\x00\x00\x00\x0d' >> "$TMPDIR/in.mid"
printf '\x00\xff\x54\x05\x01\x02\x03\x04\x05' >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/smpte-offset"
