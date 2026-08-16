source ../helpers.nu

# C: strtod consumes the longest valid float prefix; "1.5xyz" → 1.5.
# Rust's f64::parse rejects the whole string, so we walk back to find
# the longest accepted prefix.
try { ^$env.REF --cutoff=1.5xyz - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --cutoff=1.5xyz - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --cutoff=1.5xyz (strtod prefix → '1.500000')"
