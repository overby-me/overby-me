# Time Signature meta (0x58) — 4-byte payload: numerator, denominator
# (2^N), MIDI clocks per click, 32nd notes per quarter note. C prints
# `Time Signature: <num>/<2^denom>, <clocks> clocks per click, <32nds>
# notated 32nd notes per quarter note`.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"
printf '\x4d\x54\x72\x6b\x00\x00\x00\x0b' >> "$TMPDIR/in.mid"
# 6/8 time, 24 clocks/click, 8 notated 32nds/quarter
printf '\x00\xff\x58\x04\x06\x03\x18\x08' >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF"  "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual"   2>&1
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-mididump time-signature (0xFF 0x58 → 6/8 with click+32nds)"
