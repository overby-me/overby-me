source ../helpers.nu

# Sign applies to the whole hex-float value.
try { ^$env.REF --cutoff=-0x1.5p0 - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --cutoff=-0x1.5p0 - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --cutoff=-0x1.5p0 (signed hex float → '-1.312500')"
