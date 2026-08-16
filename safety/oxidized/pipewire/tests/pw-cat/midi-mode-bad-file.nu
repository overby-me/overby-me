source ../helpers.nu

# `pw-cat --midi -p <file>` switches the data type from PCM to MIDI, so
# C uses midi_file_open() instead of sf_open() and produces a "midifile:"
# error rather than the sndfile error. Verify our impl tracks the
# data-type override from the --midi flag.
try { ^$env.REF --midi -p /tmp/nonexistent-midi-file o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --midi -p /tmp/nonexistent-midi-file o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat --midi -p (uses midifile error format, not sndfile)"
