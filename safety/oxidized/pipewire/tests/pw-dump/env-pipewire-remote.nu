source ../helpers.nu

let expected = $env.TMPDIR | path join expected
let actual = $env.TMPDIR | path join actual
let e_ref = (with-env {PIPEWIRE_REMOTE: non-existent} { try { ^$env.REF o+e> $expected }; $env.LAST_EXIT_CODE })
let e_rust = (with-env {PIPEWIRE_REMOTE: non-existent} { try { ^$env.RUST o+e> $actual }; $env.LAST_EXIT_CODE })
$"exit=($e_ref)\n" | save -a --raw $expected
$"exit=($e_rust)\n" | save -a --raw $actual
compare "pw-dump with PIPEWIRE_REMOTE env (non-existent socket)"
