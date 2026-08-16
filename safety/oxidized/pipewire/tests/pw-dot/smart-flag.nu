source ../helpers.nu

# C: case 's' prints "smart option enabled" to stderr.
try { ^$env.REF --smart o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --smart o+e> ($env.TMPDIR | path join actual) }
compare "pw-dot --smart (prints 'smart option enabled' to stderr)"
