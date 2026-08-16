source ../helpers.nu

try { ^$env.REF s o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST s o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli s (alias usage)"
