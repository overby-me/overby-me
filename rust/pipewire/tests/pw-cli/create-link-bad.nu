source ../helpers.nu

try { ^$env.REF create-link 1 2 3 4 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST create-link 1 2 3 4 o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli create-link 1 2 3 4 (no daemon)"
