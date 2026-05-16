# Cue Point meta event (0x07) — also goes through the text printer
# path. Single event to avoid C's `%s` buffer-reuse bug.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"
printf '\x4d\x54\x72\x6b\x00\x00\x00\x09' >> "$TMPDIR/in.mid"
printf '\x00\xff\x07\x02Cu' >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF"  "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual"   2>&1
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-mididump cue-point-meta (0x07)"
