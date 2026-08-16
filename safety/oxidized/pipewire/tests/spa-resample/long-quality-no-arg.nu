source ../helpers.nu

try { ^$env.REF --quality o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --quality o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --quality (long form requires arg)"
