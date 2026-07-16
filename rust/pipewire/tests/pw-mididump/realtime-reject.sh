# SMF spec rejects realtime bytes (0xF8-0xFE) — C midifile.c returns
# -EINVAL on unknown status, so the loop stops without printing them.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"
printf '\x4d\x54\x72\x6b\x00\x00\x00\x07' >> "$TMPDIR/in.mid"
printf '\x00\xf8' >> "$TMPDIR/in.mid"             # delta 0, Timing Clock
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"     # delta 0, EndOfTrack

"$REF"  "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual"   2>&1
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-mididump realtime 0xF8 (rejected silently like C midifile.c)"
