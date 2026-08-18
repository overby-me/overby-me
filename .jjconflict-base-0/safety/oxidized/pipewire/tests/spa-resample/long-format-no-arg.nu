source ../helpers.nu

try { ^$env.REF --format o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --format o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --format (long form requires arg)"
