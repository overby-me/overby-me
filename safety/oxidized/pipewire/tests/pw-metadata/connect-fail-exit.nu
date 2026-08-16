source ../helpers.nu

let e_ref = with-env {PIPEWIRE_REMOTE: "non-existent"} { try { ^$env.REF o+e> ($env.TMPDIR | path join expected); 0 } catch { |e| $e.exit_code } }
let e_rust = with-env {PIPEWIRE_REMOTE: "non-existent"} { try { ^$env.RUST o+e> ($env.TMPDIR | path join actual); 0 } catch { |e| $e.exit_code } }
$"exit=($e_ref)\n" | save -a --raw ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-metadata connect-fail exits 255"
