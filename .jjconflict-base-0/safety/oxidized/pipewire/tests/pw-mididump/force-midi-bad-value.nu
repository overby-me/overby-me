source ../helpers.nu

try { ^$env.REF --force-midi=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --force-midi=foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-mididump --force-midi=foo (bad value)"
