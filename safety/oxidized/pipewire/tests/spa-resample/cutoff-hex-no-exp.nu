source ../helpers.nu

# Hex float exponent (pN) is optional: `0x1` parses as 1.0.
try { ^$env.REF --cutoff=0x1 - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --cutoff=0x1 - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --cutoff=0x1 (hex int → '1.000000')"
