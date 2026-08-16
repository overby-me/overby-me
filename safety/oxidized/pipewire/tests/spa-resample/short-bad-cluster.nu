source ../helpers.nu

try { ^$env.REF -bx o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -bx o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample -bx (short bad cluster)"
