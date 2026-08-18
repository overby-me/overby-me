source ../helpers.nu

try { ^$env.REF --list=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --list=foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-metadata --list=foo (no-arg flag)"
