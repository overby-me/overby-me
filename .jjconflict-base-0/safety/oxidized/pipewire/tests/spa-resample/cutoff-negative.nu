source ../helpers.nu

# strtod handles sign correctly; printf %f preserves it.
try { ^$env.REF --cutoff=-1.5 - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --cutoff=-1.5 - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --cutoff=-1.5 (signed decimal → '-1.500000')"
