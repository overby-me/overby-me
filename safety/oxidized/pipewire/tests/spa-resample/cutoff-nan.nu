source ../helpers.nu

# C: strtod("nan") → NaN; printf "%f" prints "nan" (lowercase). Rust's
# default Display prints "NaN" (uppercase) so we must hand-format.
try { ^$env.REF --cutoff=nan - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --cutoff=nan - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --cutoff=nan (printf %f → 'nan' lowercase)"
