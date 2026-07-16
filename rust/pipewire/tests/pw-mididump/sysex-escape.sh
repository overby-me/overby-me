# 0xF7 alone (without prior 0xF0) is the SMF "escape" sequence — raw
# bytes treated as a sysex chunk. The parser reads varlen length then
# that many payload bytes, and the printer emits `SysEx: f7 <bytes>`.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"
printf '\x4d\x54\x72\x6b\x00\x00\x00\x09' >> "$TMPDIR/in.mid"
printf '\x00\xf7\x02\xaa\xbb' >> "$TMPDIR/in.mid"  # delta 0, escape, len 2, payload AA BB
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF"  "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual"   2>&1
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-mididump sysex-escape (0xF7 escape sequence)"
