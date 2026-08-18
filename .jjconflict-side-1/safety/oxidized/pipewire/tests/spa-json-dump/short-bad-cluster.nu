source ../helpers.nu

try { ^$env.REF -bx o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -bx o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump -bx (short bad cluster)"
