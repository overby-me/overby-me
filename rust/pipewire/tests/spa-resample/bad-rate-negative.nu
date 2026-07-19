source ../helpers.nu

try { ^$env.REF --rate=-5 - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --rate=-5 - - o+e> ($env.TMPDIR | path join actual) }
"exit=0\n" | save --raw --append ($env.TMPDIR | path join expected)
"exit=0\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "spa-resample --rate=-5 (atoi -5 ≤ 0 → 'bad rate -5')"
