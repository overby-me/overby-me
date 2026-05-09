# SMF with key signature meta event.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# 0xff 0x59 0x02 sf mi
# sf = sharps/flats (-7..7), mi = 0=major 1=minor.
# 2 sharps, major (D major).
printf '\x4d\x54\x72\x6b\x00\x00\x00\x09' >> "$TMPDIR/in.mid"
printf '\x00\xff\x59\x02\x02\x00' >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/key-signature"
