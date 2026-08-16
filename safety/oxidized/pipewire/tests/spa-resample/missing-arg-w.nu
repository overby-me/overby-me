source ../helpers.nu

# spa-resample -w (short --window) requires an argument.
try { ^$env.REF -w - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -w - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample -w (missing window argument)"
