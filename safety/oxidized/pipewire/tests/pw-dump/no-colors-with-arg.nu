source ../helpers.nu

try { ^$env.REF "--no-colors=foo" o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST "--no-colors=foo" o+e> ($env.TMPDIR | path join actual) }
compare "pw-dump --no-colors=foo (no-arg flag)"
