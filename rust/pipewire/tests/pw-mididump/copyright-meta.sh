# SMF with copyright + marker + cue point meta events.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# Track body, length:
#   delta=0, 0xff 0x02 0x05 "(c)26"  = 9 bytes
#   delta=0, 0xff 0x06 0x05 "start"  = 9 bytes
#   delta=0, 0xff 0x07 0x05 "cuept"  = 9 bytes
#   delta=0, EndOfTrack              = 4 bytes
# Total = 31 bytes.
# (We use equal-length payloads to avoid C's read-past-length UB when
# its malloc'd buffer for a shorter event still holds the previous
# longer event's bytes; this would print "cuert" for a 3-byte "cue"
# after a 5-byte "start" because the unterminated payload reads into
# the leftover heap data.)
printf '\x4d\x54\x72\x6b\x00\x00\x00\x1f' >> "$TMPDIR/in.mid"
printf '\x00\xff\x02\x05(c)26' >> "$TMPDIR/in.mid"
printf '\x00\xff\x06\x05start' >> "$TMPDIR/in.mid"
printf '\x00\xff\x07\x05cuept' >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00'      >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/copyright-meta"
