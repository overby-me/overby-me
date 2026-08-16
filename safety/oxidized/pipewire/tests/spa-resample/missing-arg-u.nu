source ../helpers.nu

# spa-resample -u (short --cutoff) requires an argument.
try { ^$env.REF -u - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -u - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample -u (missing cutoff argument)"
