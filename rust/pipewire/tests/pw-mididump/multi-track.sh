# format=1 SMF with 2 tracks. Each track has a NoteOn+NoteOff.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x01\x00\x02\x01\xe0' > "$TMPDIR/in.mid"

# Track 1: 9 bytes — delta 0 NoteOn, delta 96 NoteOff, EndOfTrack.
printf '\x4d\x54\x72\x6b\x00\x00\x00\x0c' >> "$TMPDIR/in.mid"
printf '\x00\x90\x3c\x64' >> "$TMPDIR/in.mid"
printf '\x60\x80\x3c\x00' >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

# Track 2: same shape but channel 1, note D4.
printf '\x4d\x54\x72\x6b\x00\x00\x00\x0c' >> "$TMPDIR/in.mid"
printf '\x00\x91\x3e\x64' >> "$TMPDIR/in.mid"
printf '\x60\x81\x3e\x00' >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/multi-track"
