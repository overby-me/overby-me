source ../helpers.nu

try { ^$env.REF -M=ump o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -M=ump o+e> ($env.TMPDIR | path join actual) }
compare "pw-mididump -M=ump (literal '=ump' value)"
