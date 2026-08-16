source ../helpers.nu

try { ^$env.REF ($env.TMPDIR | path join nonexistent.dsf) o+e> ($env.TMPDIR | path join expected) }
let e_ref = $env.LAST_EXIT_CODE
try { ^$env.RUST ($env.TMPDIR | path join nonexistent.dsf) o+e> ($env.TMPDIR | path join actual) }
let e_rust = $env.LAST_EXIT_CODE
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
$"exit=($e_ref)\n" | save -a --raw ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-dsdplay missing-file (dsdfile: can't read)"
