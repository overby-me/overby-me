source ../helpers.nu

# `pw-cat -m -p`: short form of --midi. Equivalent to the long form.
try { ^$env.REF -m -p /tmp/nonexistent-midi-file o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -m -p /tmp/nonexistent-midi-file o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat -m -p (short --midi switches data-type → midifile error)"
