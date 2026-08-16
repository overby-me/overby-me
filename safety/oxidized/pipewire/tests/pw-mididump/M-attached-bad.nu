source ../helpers.nu

try { ^$env.REF -Mh o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -Mh o+e> ($env.TMPDIR | path join actual) }
compare "pw-mididump -Mh (attached bad value)"
