source ../helpers.nu

# spa-resample -f (short --format) requires an argument.
try { ^$env.REF -f - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -f - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample -f (missing format argument)"
