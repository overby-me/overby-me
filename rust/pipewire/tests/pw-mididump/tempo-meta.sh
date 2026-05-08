# SMF with tempo + time signature meta events.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# 0xff 0x51 0x03 (tempo: 3 bytes = 500000 us/qn = 120 BPM)
# 0xff 0x58 0x04 (time sig: 4/4, 24 clocks/click, 8 32nds/qtr)
# 0xff 0x59 0x02 (key sig: 0 sharps, major)
# 0xff 0x2f 0x00 (end of track)
body=$(printf '\x00\xff\x51\x03\x07\xa1\x20')
body="$body$(printf '\x00\xff\x58\x04\x04\x02\x18\x08')"
body="$body$(printf '\x00\xff\x59\x02\x00\x00')"
body="$body$(printf '\x00\xff\x2f\x00')"

len=$(printf "%s" "$body" | wc -c)
printf '\x4d\x54\x72\x6b' >> "$TMPDIR/in.mid"
printf "$(printf '\\x%02x\\x%02x\\x%02x\\x%02x' \
  $(( (len >> 24) & 0xff )) $(( (len >> 16) & 0xff )) \
  $(( (len >>  8) & 0xff )) $(( (len      ) & 0xff )))" >> "$TMPDIR/in.mid"
printf "%s" "$body" >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/tempo-meta"
