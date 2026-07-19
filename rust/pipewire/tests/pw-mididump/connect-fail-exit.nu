source ../helpers.nu

let ec_ref = (with-env {PIPEWIRE_REMOTE: "non-existent"} { try { ^$env.REF o+e> ($env.TMPDIR | path join expected); 0 } catch { 0 } })
let ec_rust = (with-env {PIPEWIRE_REMOTE: "non-existent"} { try { ^$env.RUST o+e> ($env.TMPDIR | path join actual); 0 } catch { 0 } })
$"exit=($ec_ref)\n" | save --raw --append ($env.TMPDIR | path join expected)
$"exit=($ec_rust)\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "pw-mididump connect-fail exits 255"
