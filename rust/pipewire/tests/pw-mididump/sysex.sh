# SMF with a SysEx (System Exclusive) event.
# Avoid null bytes in shell variables — write body directly to file.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# Track body, length 13 bytes:
#   8 bytes: SysEx event with 5-byte payload (length-byte + 4 data bytes)
#   4 bytes: EndOfTrack meta
#   delta-time bytes: 1+1 = 2
# Total = 13. We hard-code MTrk length.
printf '\x4d\x54\x72\x6b\x00\x00\x00\x0d' >> "$TMPDIR/in.mid"
# delta=0, 0xf0, length=4, data 0x43 0x12 0x01 0xf7 (no internal NULs).
printf '\x00\xf0\x04\x43\x12\x01\xf7' >> "$TMPDIR/in.mid"
# delta=0, EndOfTrack
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/sysex"
