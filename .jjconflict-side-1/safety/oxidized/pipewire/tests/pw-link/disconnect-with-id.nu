source ../helpers.nu

try { ^$env.REF -d 1 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -d 1 o+e> ($env.TMPDIR | path join actual) }
compare "pw-link -d 1 (disconnect by id)"
