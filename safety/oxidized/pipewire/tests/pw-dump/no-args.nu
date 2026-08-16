source ../helpers.nu

let expected = $env.TMPDIR | path join expected
let actual = $env.TMPDIR | path join actual
try { ^$env.REF o+e> $expected }
let e_ref = $env.LAST_EXIT_CODE
try { ^$env.RUST o+e> $actual }
let e_rust = $env.LAST_EXIT_CODE
$"exit=($e_ref)\n" | save -a --raw $expected
$"exit=($e_rust)\n" | save -a --raw $actual
compare "pw-dump no args (sandbox: connect-fail)"
