source ../helpers.nu

# C: case 'd' prints "detail option enabled" to stderr.
try { ^$env.REF --detail o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --detail o+e> ($env.TMPDIR | path join actual) }
compare "pw-dot --detail (prints 'detail option enabled' to stderr)"
