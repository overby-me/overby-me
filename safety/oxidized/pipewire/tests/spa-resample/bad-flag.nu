source ../helpers.nu

try { ^$env.REF --bad o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --bad o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample/bad-flag"
