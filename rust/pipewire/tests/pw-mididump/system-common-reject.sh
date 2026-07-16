# SMF spec: MTrk events are limited to channel messages (0x80-0xEF),
# sysex (0xF0/0xF7) and meta (0xFF). System Common (0xF1-0xF6) and
# Realtime (0xF8-0xFE) bytes hit C's `default: return -EINVAL` arm in
# midifile.c — the event loop stops without printing them.
#
# Build an SMF with a stray 0xF1 (MIDI Time Code Quarter Frame) byte
# and verify both tools only emit the "opened" header.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"
printf '\x4d\x54\x72\x6b\x00\x00\x00\x08' >> "$TMPDIR/in.mid"
printf '\x00\xf1\x42' >> "$TMPDIR/in.mid"          # delta 0, MTC Quarter Frame
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"      # delta 0, EndOfTrack

"$REF"  "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual"   2>&1
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-mididump system-common 0xF1 (rejected silently like C midifile.c)"
