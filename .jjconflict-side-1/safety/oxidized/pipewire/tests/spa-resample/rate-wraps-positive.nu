source ../helpers.nu

# 4294967297 = 2^32 + 1; atoi wraps to 1 (positive) → no "bad rate".
# Proceeds to filename check / sndfile open.
try { ^$env.REF --rate=4294967297 - - o+e> ($env.TMPDIR | path join expected) }
let e_ref = $env.LAST_EXIT_CODE
try { ^$env.RUST --rate=4294967297 - - o+e> ($env.TMPDIR | path join actual) }
let e_rust = $env.LAST_EXIT_CODE
$"exit=($e_ref)\n" | save --raw --append ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "spa-resample --rate=4294967297 (wraps to 1 via atoi, not 'bad rate')"
