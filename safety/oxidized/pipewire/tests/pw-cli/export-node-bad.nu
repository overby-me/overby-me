source ../helpers.nu

try { ^$env.REF export-node 1 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST export-node 1 o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli export-node 1 (no daemon)"
