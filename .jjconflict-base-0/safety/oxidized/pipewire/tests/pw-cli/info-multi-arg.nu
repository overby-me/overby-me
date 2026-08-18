source ../helpers.nu

try { ^$env.REF i 0 garbage o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST i 0 garbage o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli i 0 garbage (multi-arg join semantics)"
