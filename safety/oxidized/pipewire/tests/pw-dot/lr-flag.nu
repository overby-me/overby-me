source ../helpers.nu

# C: case 'L' prints "set rank direction to LR" to stderr.
try { ^$env.REF --lr o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --lr o+e> ($env.TMPDIR | path join actual) }
compare "pw-dot --lr (prints 'set rank direction to LR' to stderr)"
