source ../helpers.nu

try { ^$env.REF "--recurse=foo" o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST "--recurse=foo" o+e> ($env.TMPDIR | path join actual) }
compare "pw-config --recurse=foo (no-arg flag)"
