# pw-mididump on a tiny format-0 SMF: NoteOn → NoteOff at 120 ticks
# (= 0.125s at 480 division / default 120 BPM tempo).

# SMF bytes: MThd + format=0 + 1 track + division=480, MTrk with NoteOn
# (0x90 60 100), NoteOff (0x80 60 0), EndOfTrack.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"
printf '\x4d\x54\x72\x6b\x00\x00\x00\x0d' >> "$TMPDIR/in.mid"
printf '\x00\x90\x3c\x64' >> "$TMPDIR/in.mid"     # delta 0, NoteOn C4 vel 100
printf '\x78\x80\x3c\x00' >> "$TMPDIR/in.mid"     # delta 120, NoteOff C4
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"     # delta 0, EndOfTrack

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/basic"
