# Program Change (0xC0) — channel event with 1 data byte. The C tool
# (midievent.c) maps program numbers to GM instrument names.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"
printf '\x4d\x54\x72\x6b\x00\x00\x00\x09' >> "$TMPDIR/in.mid"
printf '\x00\xc0\x01' >> "$TMPDIR/in.mid"          # delta 0, Program Change ch 1, program 1 (Bright Acoustic)
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"      # delta 0, EndOfTrack

"$REF"  "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual"   2>&1
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-mididump program-change (0xC0 with GM instrument name)"
