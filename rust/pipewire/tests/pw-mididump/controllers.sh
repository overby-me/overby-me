# SMF with controller, program-change, pitch-bend.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# Track body:
#   delta=0, 0xb0 0x07 0x40    = 4 bytes (CC 7 = 64)
#   delta=0, 0xc0 0x28         = 3 bytes (Program 40)
#   delta=0, 0xe0 0x00 0x40    = 4 bytes (Pitch Bend center)
#   delta=0, EndOfTrack        = 4 bytes
# Total = 15 bytes (0x0f).
printf '\x4d\x54\x72\x6b\x00\x00\x00\x0f' >> "$TMPDIR/in.mid"
printf '\x00\xb0\x07\x40'     >> "$TMPDIR/in.mid"
printf '\x00\xc0\x28'         >> "$TMPDIR/in.mid"
printf '\x00\xe0\x00\x40'     >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00'     >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/controllers"
