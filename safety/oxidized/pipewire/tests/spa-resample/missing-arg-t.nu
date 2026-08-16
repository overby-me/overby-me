source ../helpers.nu

# spa-resample -t (short --taps) requires an argument.
try { ^$env.REF -t - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -t - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample -t (missing taps argument)"
