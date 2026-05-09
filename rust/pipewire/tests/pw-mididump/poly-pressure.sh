# SMF with polyphonic key pressure (aftertouch) and channel pressure.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# Track body, length 11:
#   4 bytes: delta=0, 0xa0 0x3c 0x40 = poly key pressure ch 0 note 60 = 64
#   3 bytes: delta=0, 0xd0 0x40    = channel pressure ch 0 = 64
#   4 bytes: delta=0, EndOfTrack
printf '\x4d\x54\x72\x6b\x00\x00\x00\x0b' >> "$TMPDIR/in.mid"
printf '\x00\xa0\x3c\x40' >> "$TMPDIR/in.mid"
printf '\x00\xd0\x40'     >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/poly-pressure"
