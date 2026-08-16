source ../helpers.nu

# C printf %f respects NaN signbit: '-nan' for a NaN with sign bit set.
# Rust's Display always prints 'NaN' regardless of sign, so we test
# both branches of our hand-formatter.
try { ^$env.REF --cutoff=-nan - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --cutoff=-nan - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --cutoff=-nan (printf %f → '-nan' with signbit)"
