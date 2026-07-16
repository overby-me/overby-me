# SMF with division=0 (invalid per spec, but produced by some tools).
# C's midifile divides by info.division giving NaN/inf; printf "%f"
# emits "nan"/"-nan" (signed). Our impl now matches C's NaN
# propagation rather than clamping to 0.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x00\x00' > "$TMPDIR/in.mid"
printf '\x4d\x54\x72\x6b\x00\x00\x00\x04' >> "$TMPDIR/in.mid"
printf '\x00\xff\x2f\x00' >> "$TMPDIR/in.mid"

"$REF"  "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual"   2>&1
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
compare "pw-mididump division-zero (sec divides by 0 → nan)"
