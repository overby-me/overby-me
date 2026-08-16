source ../helpers.nu

# spa-resample -p (short --param) requires an argument.
try { ^$env.REF -p - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -p - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample -p (missing param argument)"
