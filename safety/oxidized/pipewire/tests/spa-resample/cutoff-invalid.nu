source ../helpers.nu

# C: strtod("invalid", NULL) → 0.0, then printf "0.000000".
try { ^$env.REF --cutoff=invalid - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --cutoff=invalid - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --cutoff=invalid (strtod fails → '0.000000')"
