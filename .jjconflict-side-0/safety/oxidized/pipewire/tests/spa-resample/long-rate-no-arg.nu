source ../helpers.nu

try { ^$env.REF --rate o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --rate o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --rate (long form requires arg)"
