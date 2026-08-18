source ../helpers.nu

try { ^$env.REF --lr=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --lr=foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-dot --lr=foo (no-arg flag)"
