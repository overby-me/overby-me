# Key Signature meta (0xFF 0x59) with a "negative" sf byte. C reads
# meta[0] as int (implicit u8→int conversion = positive); -7 becomes
# 249, classified "sharps", abs(249)=249, table lookup is clamped to
# 18 → "Unknown major". Our impl now matches the u8-as-int quirk.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"
printf '\x4d\x54\x72\x6b\x00\x00\x00\x0a' >> "$TMPDIR/in.mid"
printf '\x00\xff\x59\x02\xf9\x00' >> "$TMPDIR/in.mid"   # sf=-7 (0xF9), major
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF"  "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual"   2>&1
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-mididump key-sig-out-of-range (negative sf byte → '249 sharps: Unknown major')"
