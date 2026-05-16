# `pw-cat --midi -p <file>` switches the data type from PCM to MIDI, so
# C uses midi_file_open() instead of sf_open() and produces a "midifile:"
# error rather than the sndfile error. Verify our impl tracks the
# data-type override from the --midi flag.
"$REF"  --midi -p /tmp/nonexistent-midi-file > "$TMPDIR/expected" 2>&1 || true
"$RUST" --midi -p /tmp/nonexistent-midi-file > "$TMPDIR/actual"   2>&1 || true
compare "pw-cat --midi -p (uses midifile error format, not sndfile)"
