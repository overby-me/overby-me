source ../helpers.nu

# C: strtod("inf") → INFINITY; printf "%f" of inf → "inf" (no '.000000').
# Rust's default Display for f64::INFINITY prints "inf" too, so this
# matches even with the generic {:.6} formatter.
try { ^$env.REF --cutoff=inf - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --cutoff=inf - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --cutoff=inf (printf %f → 'inf')"
