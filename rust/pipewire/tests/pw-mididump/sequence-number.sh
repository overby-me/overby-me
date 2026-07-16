# SMF with sequence number meta event (FF 00 02 ssss).
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# 0xff 0x00 0x02 ss ss (sequence number = 0x0042 = 66)
printf '\x4d\x54\x72\x6b\x00\x00\x00\x0a' >> "$TMPDIR/in.mid"
printf '\x00\xff\x00\x02\x00\x42' >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/sequence-number"
