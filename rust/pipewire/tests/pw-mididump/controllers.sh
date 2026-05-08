# pw-mididump on a SMF with controller, program-change, pitch-bend.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# Track body: build incrementally so we can size MTrk correctly.
body=$(printf '\x00\xb0\x07\x40')                 # CC 7 (Volume coarse)=64
body="$body$(printf '\x00\xc0\x28')"              # Program 40 (Violin)
body="$body$(printf '\x00\xe0\x00\x40')"          # Pitch Bend center
body="$body$(printf '\x00\xff\x2f\x00')"          # End of track

len=$(printf "%s" "$body" | wc -c)
# 4-byte big-endian length.
printf '\x4d\x54\x72\x6b' >> "$TMPDIR/in.mid"
printf "$(printf '\\x%02x\\x%02x\\x%02x\\x%02x' \
  $(( (len >> 24) & 0xff )) $(( (len >> 16) & 0xff )) \
  $(( (len >>  8) & 0xff )) $(( (len      ) & 0xff )))" >> "$TMPDIR/in.mid"
printf "%s" "$body" >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/controllers"
