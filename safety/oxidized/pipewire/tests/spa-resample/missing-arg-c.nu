source ../helpers.nu

try { ^$env.REF -v -c o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -v -c o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample/missing-arg-c"
