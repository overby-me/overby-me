source ../helpers.nu

# C: strtod accepts hex floats `0x<mantissa>p<binary exponent>`.
# 0x1.5p0 = (1 + 5/16) * 2^0 = 1.3125 → printf %f = '1.312500'.
try { ^$env.REF --cutoff=0x1.5p0 - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --cutoff=0x1.5p0 - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --cutoff=0x1.5p0 (hex float → '1.312500')"
