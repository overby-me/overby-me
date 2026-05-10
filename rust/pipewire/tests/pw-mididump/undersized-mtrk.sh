# MTrk size header says 7 bytes but the body actually has 11 (Tempo +
# EOT). C reads exactly 7 bytes per the header — only the Tempo event,
# not the EOT.
printf '\x4D\x54\x68\x64\x00\x00\x00\x06\x00\x00\x00\x01\x00\x60\x4D\x54\x72\x6B\x00\x00\x00\x07\x00\xFF\x51\x03\x07\xA1\x20\x00\xFF\x2F\x00' > "$TMPDIR/tempo.mid"
"$REF" "$TMPDIR/tempo.mid" </dev/null > "$TMPDIR/expected" 2>&1 || true
e_ref=$?
"$RUST" "$TMPDIR/tempo.mid" </dev/null > "$TMPDIR/actual" 2>&1 || true
e_rust=$?
sed -i "s|$TMPDIR|TMPDIR|g" "$TMPDIR/expected" "$TMPDIR/actual"
echo "exit=$e_ref" >> "$TMPDIR/expected"
echo "exit=$e_rust" >> "$TMPDIR/actual"
compare "pw-mididump MTrk size truncates body (read exactly size bytes)"
