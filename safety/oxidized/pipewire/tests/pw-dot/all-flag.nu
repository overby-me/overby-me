source ../helpers.nu

# C: case 'a' prints "all option enabled" to stderr from option-parse.
try { ^$env.REF --all o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --all o+e> ($env.TMPDIR | path join actual) }
compare "pw-dot --all (prints 'all option enabled' to stderr)"
