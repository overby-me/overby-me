source ../helpers.nu

try { ^$env.REF --quality=999999999999 - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --quality=999999999999 - - o+e> ($env.TMPDIR | path join actual) }
"exit=0\n" | save --raw --append ($env.TMPDIR | path join expected)
"exit=0\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "spa-resample --quality=999999999999 (overflow → 'bad quality <orig>')"
