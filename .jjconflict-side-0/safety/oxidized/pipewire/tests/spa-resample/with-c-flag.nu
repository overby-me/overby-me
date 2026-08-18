source ../helpers.nu

try { ^$env.REF -c 0 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -c 0 o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample/with-c-flag"
