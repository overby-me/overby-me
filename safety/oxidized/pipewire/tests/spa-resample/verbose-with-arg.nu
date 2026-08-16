source ../helpers.nu

try { ^$env.REF --verbose=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --verbose=foo o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --verbose=foo (no-arg flag)"
