source ../helpers.nu

try { ^$env.REF i o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST i o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli i (alias, no args)"
