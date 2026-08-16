source ../helpers.nu

# getopt: required-arg short flag accepts attached value `-u<val>`.
try { ^$env.REF -u0.75 - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -u0.75 - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample -u0.75 (attached short value → '0.750000')"
