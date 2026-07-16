# Text meta events: track-name, instrument, lyric.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# Track body:
#   delta=0, 0xff 0x03 0x04 "test"     = 8 bytes
#   delta=0, 0xff 0x04 0x05 "piano"    = 9 bytes
#   delta=0, 0xff 0x05 0x02 "la"       = 6 bytes
#   delta=0, EndOfTrack                = 4 bytes
# Total = 27 bytes (0x1b).
printf '\x4d\x54\x72\x6b\x00\x00\x00\x1b' >> "$TMPDIR/in.mid"
printf '\x00\xff\x03\x04test'             >> "$TMPDIR/in.mid"
printf '\x00\xff\x04\x05piano'            >> "$TMPDIR/in.mid"
printf '\x00\xff\x05\x02la'               >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00'                 >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/text-meta"
