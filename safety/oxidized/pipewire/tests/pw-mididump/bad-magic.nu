source ../helpers.nu

0x[4e 4f 50 45] | save -f --raw ($env.TMPDIR | path join bad.mid)
let ec_ref = (try { ^$env.REF ($env.TMPDIR | path join bad.mid) o+e> ($env.TMPDIR | path join expected); 0 } catch { 0 })
let ec_rust = (try { ^$env.RUST ($env.TMPDIR | path join bad.mid) o+e> ($env.TMPDIR | path join actual); 0 } catch { 0 })
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
$"exit=($ec_ref)\n" | save --raw --append ($env.TMPDIR | path join expected)
$"exit=($ec_rust)\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "pw-mididump bad magic (no MThd → Invalid argument)"
