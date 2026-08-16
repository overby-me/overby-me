source ../helpers.nu

# getopt: short flag with separated value `-u <val>`.
try { ^$env.REF -u 0.42 - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -u 0.42 - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample -u 0.42 (separated short value → '0.420000')"
