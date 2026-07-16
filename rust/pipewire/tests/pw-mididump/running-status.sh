# SMF using running status: status byte 0x90 (NoteOn ch 0) sent once,
# subsequent events omit it.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# Track body:
#   delta=0, 0x90 60 100   = 4 bytes (NoteOn full)
#   delta=120, 64 100      = 3 bytes (running status)
#   delta=0, 60 0          = 3 bytes (running status)
#   delta=0, EndOfTrack    = 4 bytes
# Total = 14 bytes (0x0e).
printf '\x4d\x54\x72\x6b\x00\x00\x00\x0e' >> "$TMPDIR/in.mid"
printf '\x00\x90\x3c\x64' >> "$TMPDIR/in.mid"
printf '\x78\x40\x64'     >> "$TMPDIR/in.mid"
printf '\x00\x3c\x00'     >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/running-status"
