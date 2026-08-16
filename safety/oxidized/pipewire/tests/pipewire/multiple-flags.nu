source ../helpers.nu

try { ^$env.REF -v -V o+e> ($env.TMPDIR | path join expected) }
let e_ref = $env.LAST_EXIT_CODE
try { ^$env.RUST -v -V o+e> ($env.TMPDIR | path join actual) }
let e_rust = $env.LAST_EXIT_CODE
$"exit=($e_ref)\n" | save -a --raw ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pipewire -v -V (verbose then version short-circuits)"
