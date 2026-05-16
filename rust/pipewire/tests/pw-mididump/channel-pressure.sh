# Channel Pressure / Aftertouch (0xD0) — channel event with 1 data
# byte (the pressure value). Distinct from polyphonic key pressure
# (0xA0) which has 2 data bytes (note + pressure).
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"
printf '\x4d\x54\x72\x6b\x00\x00\x00\x08' >> "$TMPDIR/in.mid"
printf '\x00\xd0\x64' >> "$TMPDIR/in.mid"          # delta 0, Channel Pressure ch 1, pressure 100
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"      # delta 0, EndOfTrack

"$REF"  "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual"   2>&1
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-mididump channel-pressure (0xD0 with 1 data byte)"
