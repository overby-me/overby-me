source ../helpers.nu

let ec_ref = (try { ^$env.REF --help=value o+e> ($env.TMPDIR | path join expected); 0 } catch { 0 })
let ec_rust = (try { ^$env.RUST --help=value o+e> ($env.TMPDIR | path join actual); 0 } catch { 0 })
$"exit=($ec_ref)\n" | save --raw --append ($env.TMPDIR | path join expected)
$"exit=($ec_rust)\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "pw-mididump --help=value exits 255"
