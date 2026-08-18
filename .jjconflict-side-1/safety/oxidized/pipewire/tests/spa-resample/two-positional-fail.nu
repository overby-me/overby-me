source ../helpers.nu

try { ^$env.REF /tmp/nonexistent /tmp/out.wav o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST /tmp/nonexistent /tmp/out.wav o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample two-positional fail (sndfile error)"
