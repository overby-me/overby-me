source ../helpers.nu

try { ^$env.REF --all=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --all=foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-dot --all=foo (no-arg flag)"
