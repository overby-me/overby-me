source ../helpers.nu

# spa-resample -q (short --quality) requires an argument.
try { ^$env.REF -q - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -q - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample -q (missing quality argument)"
