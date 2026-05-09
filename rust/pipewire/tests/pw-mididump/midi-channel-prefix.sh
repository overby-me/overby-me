# SMF with MIDI channel prefix meta event.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# 0xff 0x20 0x01 cc (MIDI channel prefix)
# This binds subsequent meta events to channel cc.
printf '\x4d\x54\x72\x6b\x00\x00\x00\x08' >> "$TMPDIR/in.mid"
printf '\x00\xff\x20\x01\x05' >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/midi-channel-prefix"
