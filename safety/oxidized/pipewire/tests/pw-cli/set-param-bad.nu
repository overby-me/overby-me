source ../helpers.nu

try { ^$env.REF set-param 0 0 0 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST set-param 0 0 0 o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli set-param 0 0 0 (no daemon)"
