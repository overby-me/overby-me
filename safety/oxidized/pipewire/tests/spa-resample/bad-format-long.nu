source ../helpers.nu

try { ^$env.REF --format=invalid - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --format=invalid - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --format=invalid (invalid format long form)"
