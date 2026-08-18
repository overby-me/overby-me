source ../helpers.nu

try { ^$env.REF --no-colors=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --no-colors=foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-mon --no-colors=foo (no-arg flag)"
