# `pw-cat -m -p` — short form of --midi. Equivalent to the long form.
"$REF"  -m -p /tmp/nonexistent-midi-file > "$TMPDIR/expected" 2>&1 || true
"$RUST" -m -p /tmp/nonexistent-midi-file > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat -m -p (short --midi switches data-type → midifile error)"
