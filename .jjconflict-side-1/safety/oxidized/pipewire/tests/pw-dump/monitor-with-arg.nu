source ../helpers.nu

try { ^$env.REF "--monitor=foo" o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST "--monitor=foo" o+e> ($env.TMPDIR | path join actual) }
compare "pw-dump --monitor=foo (no-arg flag)"
