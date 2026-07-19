source ../helpers.nu

# getopt_long: long flag with separated value `--cutoff <val>`.
try { ^$env.REF --cutoff 0.99 - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --cutoff 0.99 - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --cutoff 0.99 (separated long value → '0.990000')"
