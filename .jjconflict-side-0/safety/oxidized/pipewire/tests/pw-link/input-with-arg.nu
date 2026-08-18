source ../helpers.nu

try { ^$env.REF --input=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --input=foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-link --input=foo (no-arg flag rejects value)"
