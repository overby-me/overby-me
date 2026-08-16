source ../helpers.nu

try { ^$env.REF --rate=0 - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --rate=0 - - o+e> ($env.TMPDIR | path join actual) }
"exit=0\n" | save --raw --append ($env.TMPDIR | path join expected)
"exit=0\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "spa-resample --rate=0 (atoi gives 0 → 'bad rate 0')"
