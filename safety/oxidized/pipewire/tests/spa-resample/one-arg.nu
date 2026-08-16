source ../helpers.nu

try { ^$env.REF foo.wav o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST foo.wav o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample/one-arg"
