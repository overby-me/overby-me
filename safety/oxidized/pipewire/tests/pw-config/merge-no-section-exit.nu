source ../helpers.nu

try { ^$env.REF merge o+e> ($env.TMPDIR | path join expected) }
let e_ref = $env.LAST_EXIT_CODE
try { ^$env.RUST merge o+e> ($env.TMPDIR | path join actual) }
let e_rust = $env.LAST_EXIT_CODE
$"exit=($e_ref)\n" | save --raw --append ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "pw-config merge (no section → exit 234)"
