# SMF using running status: status byte 0x90 (NoteOn ch 0) is sent once,
# then subsequent events omit it. The parser must apply the running status.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# Track body:
# - delta 0, status 0x90, note 60, vel 100
# - delta 120, no status (running), note 64, vel 100  (NoteOn ch 0 E4 vel 100)
# - delta 0, no status (running), note 60, vel 0  (NoteOn ch 0 C4 vel 0 — equivalent NoteOff)
# - delta 0, EndOfTrack
body=$(printf '\x00\x90\x3c\x64')
body="$body$(printf '\x78\x40\x64')"
body="$body$(printf '\x00\x3c\x00')"
body="$body$(printf '\x00\xff\x2f\x00')"

len=$(printf "%s" "$body" | wc -c)
printf '\x4d\x54\x72\x6b' >> "$TMPDIR/in.mid"
printf "$(printf '\\x%02x\\x%02x\\x%02x\\x%02x' \
  $(( (len >> 24) & 0xff )) $(( (len >> 16) & 0xff )) \
  $(( (len >>  8) & 0xff )) $(( (len      ) & 0xff )))" >> "$TMPDIR/in.mid"
printf "%s" "$body" >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/running-status"
