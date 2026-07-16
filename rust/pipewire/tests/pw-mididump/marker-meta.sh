# Marker meta event (0x06) — same printer path as Text/Copyright/Track/
# Instrument/Lyric/Cue (case 0x01..0x09 in midievent.c). Single event
# only so we don't trigger C's `%s` buffer-reuse bug that reads stale
# bytes past the payload boundary on multi-event meta sequences.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"
printf '\x4d\x54\x72\x6b\x00\x00\x00\x0c' >> "$TMPDIR/in.mid"
printf '\x00\xff\x06\x05Verse' >> "$TMPDIR/in.mid"     # Marker "Verse"
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF"  "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual"   2>&1
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-mididump marker-meta (0x06)"
