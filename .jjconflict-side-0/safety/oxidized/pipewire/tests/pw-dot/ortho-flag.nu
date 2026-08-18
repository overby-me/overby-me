source ../helpers.nu

# C: case '9' prints "orthogonal edges enabled" to stderr.
try { ^$env.REF --90 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --90 o+e> ($env.TMPDIR | path join actual) }
compare "pw-dot --90 (prints 'orthogonal edges enabled' to stderr)"
