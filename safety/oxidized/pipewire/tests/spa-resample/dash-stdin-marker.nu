source ../helpers.nu

try { ^$env.REF - /tmp/out.wav o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST - /tmp/out.wav o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample - /tmp/out.wav (stdin marker → Format not recognised)"
