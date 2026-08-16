source ../helpers.nu

let e_ref = (with-env { PIPEWIRE_REMOTE: "non-existent" } { try { ^$env.REF o+e> ($env.TMPDIR | path join expected) }; $env.LAST_EXIT_CODE })
let e_rust = (with-env { PIPEWIRE_REMOTE: "non-existent" } { try { ^$env.RUST o+e> ($env.TMPDIR | path join actual) }; $env.LAST_EXIT_CODE })
$"exit=($e_ref)\n" | save -a --raw ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-top connect-fail exits 255"
