# Text meta events: track-name, instrument, lyric.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# 0xff 0x03 ... (Sequence/Track Name)
# 0xff 0x04 ... (Instrument)
# 0xff 0x05 ... (Lyric)
# 0xff 0x2f 0x00 (end)
body=$(printf '\x00\xff\x03\x04test')           # name = "test"
body="$body$(printf '\x00\xff\x04\x05piano')"   # instrument = "piano"
body="$body$(printf '\x00\xff\x05\x02la')"      # lyric = "la"
body="$body$(printf '\x00\xff\x2f\x00')"

len=$(printf "%s" "$body" | wc -c)
printf '\x4d\x54\x72\x6b' >> "$TMPDIR/in.mid"
printf "$(printf '\\x%02x\\x%02x\\x%02x\\x%02x' \
  $(( (len >> 24) & 0xff )) $(( (len >> 16) & 0xff )) \
  $(( (len >>  8) & 0xff )) $(( (len      ) & 0xff )))" >> "$TMPDIR/in.mid"
printf "%s" "$body" >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/text-meta"
