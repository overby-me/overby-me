source ../helpers.nu

# atoi takes leading digits only: "44100abc" → 44100, no "bad rate".
try { ^$env.REF --rate=44100abc - - o+e> ($env.TMPDIR | path join expected) }
let e_ref = $env.LAST_EXIT_CODE
try { ^$env.RUST --rate=44100abc - - o+e> ($env.TMPDIR | path join actual) }
let e_rust = $env.LAST_EXIT_CODE
$"exit=($e_ref)\n" | save --raw --append ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "spa-resample --rate=44100abc (atoi reads 44100, not 'bad rate')"
