source ../helpers.nu

try { ^$env.REF -vv o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -vv o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample/double-v"
