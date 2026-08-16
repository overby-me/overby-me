source ../helpers.nu

try { ^$env.REF create-device foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST create-device foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli create-device foo (no daemon)"
