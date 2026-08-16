source ../helpers.nu

try { ^$env.REF -p /tmp/nonexistent.wav o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -p /tmp/nonexistent.wav o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat -p /tmp/nonexistent.wav (sndfile error)"
