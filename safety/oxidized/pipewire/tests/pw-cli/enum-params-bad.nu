source ../helpers.nu

try { ^$env.REF enum-params 0 0 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST enum-params 0 0 o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli enum-params 0 0 (no daemon)"
