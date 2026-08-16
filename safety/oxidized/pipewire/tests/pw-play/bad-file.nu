source ../helpers.nu

try { ^$env.REF /tmp/nonexistent.wav o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST /tmp/nonexistent.wav o+e> ($env.TMPDIR | path join actual) }
compare "pw-play /tmp/nonexistent.wav (sndfile error)"
