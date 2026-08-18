source ../helpers.nu

try { ^$env.REF -r=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -r=foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli -r=foo (short with =value)"
