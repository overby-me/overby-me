source ../helpers.nu

# pw-mon --color= (with `=` and empty value). C's getopt sets optarg
# to the empty string in this case; the value doesn't match any of
# 'auto'/'never'/'always' so we hit the "Invalid color: " error.
# Bare `--color` (no `=`) is different: optarg is NULL there and the
# C code's `optarg == NULL || strcmp(optarg, "auto")==0` short-circuits.
try { ^$env.REF --color= o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --color= o+e> ($env.TMPDIR | path join actual) }
compare "pw-mon --color= (empty value rejected like C)"
