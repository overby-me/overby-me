source ../helpers.nu

try { ^$env.REF get-permissions 0 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST get-permissions 0 o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli get-permissions 0 (no daemon)"
