source ../helpers.nu

# atoi("") = 0, not <0 → not 'bad quality'.
try { ^$env.REF --quality= - - o+e> ($env.TMPDIR | path join expected) }
let e_ref = $env.LAST_EXIT_CODE
try { ^$env.RUST --quality= - - o+e> ($env.TMPDIR | path join actual) }
let e_rust = $env.LAST_EXIT_CODE
$"exit=($e_ref)\n" | save --raw --append ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "spa-resample --quality= (atoi empty = 0, not <0 → no 'bad quality')"
