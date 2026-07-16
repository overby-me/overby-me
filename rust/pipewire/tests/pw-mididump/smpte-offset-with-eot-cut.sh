# MTrk size includes the SMPTE event + start of EOT marker (just 0xFF),
# leaving an incomplete meta event at the end. C tolerates this and
# synthesizes EOT; we must match that behavior.
printf '\x4D\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x00\x60\x4D\x54\x72\x6B\x00\x00\x00\x0B\x00\xFF\x54\x05\x01\x02\x03\x04\x05\x00\xFF\x2F\x00' > "$TMPDIR/smpte.mid"
"$REF" "$TMPDIR/smpte.mid" </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" "$TMPDIR/smpte.mid" </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-mididump SMPTE offset + partial EOT (tolerant)"
