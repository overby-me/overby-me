# Sequencer Specific meta event (0xFF 0x7F) — printer uses dump_mem
# with "Sequencer" label, emitting the raw bytes as hex.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"
printf '\x4d\x54\x72\x6b\x00\x00\x00\x09' >> "$TMPDIR/in.mid"
printf '\x00\xff\x7f\x02\xaa\xbb' >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF"  "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual"   2>&1
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-mididump sequencer-specific (0xFF 0x7F → 'Sequencer: <hex>')"
