source ../helpers.nu

try { ^$env.REF --batch-mode=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --batch-mode=foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-top --batch-mode=foo (no-arg flag)"
