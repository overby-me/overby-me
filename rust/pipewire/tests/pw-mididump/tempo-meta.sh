# SMF with tempo + time signature meta events. Avoid bash $() command
# substitution which strips NULs — write bytes directly to the file.
printf '\x4d\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x01\xe0' > "$TMPDIR/in.mid"

# Track body, length 25:
#   delta=0, 0xff 0x51 0x03 tempo (3 bytes)  = 7 bytes
#   delta=0, 0xff 0x58 0x04 time-sig (4)     = 8 bytes
#   delta=0, 0xff 0x59 0x02 key-sig (2)      = 6 bytes
#   delta=0, EndOfTrack                      = 4 bytes
# Total = 25 bytes (0x19).
printf '\x4d\x54\x72\x6b\x00\x00\x00\x19' >> "$TMPDIR/in.mid"
# tempo: 500000 us/qn = 120 BPM
printf '\x00\xff\x51\x03\x07\xa1\x20'       >> "$TMPDIR/in.mid"
# time sig: 4/4, 24 clocks/click, 8 32nds/qtr
printf '\x00\xff\x58\x04\x04\x02\x18\x08'   >> "$TMPDIR/in.mid"
# key sig: 0 sharps, major
printf '\x00\xff\x59\x02\x00\x00'           >> "$TMPDIR/in.mid"
# end
printf '\x00\xff\x2f\x00'                   >> "$TMPDIR/in.mid"

"$REF" "$TMPDIR/in.mid" > "$TMPDIR/expected" 2>&1
"$RUST" "$TMPDIR/in.mid" > "$TMPDIR/actual" 2>&1
compare "pw-mididump/tempo-meta"
