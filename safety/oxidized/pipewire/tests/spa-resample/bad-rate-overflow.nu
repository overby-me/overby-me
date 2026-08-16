source ../helpers.nu

try { ^$env.REF --rate=999999999999 - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --rate=999999999999 - - o+e> ($env.TMPDIR | path join actual) }
"exit=0\n" | save --raw --append ($env.TMPDIR | path join expected)
"exit=0\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "spa-resample --rate=999999999999 (i32 overflow → 'bad rate <orig>')"
