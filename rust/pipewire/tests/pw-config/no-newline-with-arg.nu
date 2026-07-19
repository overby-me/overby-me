source ../helpers.nu

try { ^$env.REF "--no-newline=foo" o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST "--no-newline=foo" o+e> ($env.TMPDIR | path join actual) }
compare "pw-config --no-newline=foo (no-arg flag)"
