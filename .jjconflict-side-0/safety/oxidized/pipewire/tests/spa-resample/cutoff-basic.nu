source ../helpers.nu

# C: case 'u' → strtod(optarg, NULL) then fprintf(stderr, "%f\n", val).
try { ^$env.REF --cutoff=0.5 - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --cutoff=0.5 - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --cutoff=0.5 (strtod → fprintf %f = '0.500000')"
