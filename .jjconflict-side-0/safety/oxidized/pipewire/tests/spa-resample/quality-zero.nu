source ../helpers.nu

# C: ret = atoi("0") = 0, NOT < 0 → no "bad quality". Proceeds to file open.
try { ^$env.REF --quality=0 - - o+e> ($env.TMPDIR | path join expected) }
let e_ref = $env.LAST_EXIT_CODE
try { ^$env.RUST --quality=0 - - o+e> ($env.TMPDIR | path join actual) }
let e_rust = $env.LAST_EXIT_CODE
$"exit=($e_ref)\n" | save --raw --append ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "spa-resample --quality=0 (atoi 0 is not <0 → file open error, not 'bad quality')"
