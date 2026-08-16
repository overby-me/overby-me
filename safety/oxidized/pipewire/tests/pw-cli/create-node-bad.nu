source ../helpers.nu

try { ^$env.REF create-node foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST create-node foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli create-node foo (no daemon)"
