source ../helpers.nu

0x[4d 54 68 64 00 00 00 06 00 00 00 01 00 60 4d 54 72 6b 00 00 00 00] | save -f --raw ($env.TMPDIR | path join empty.mid)
let ec_ref = (try { ^$env.REF ($env.TMPDIR | path join empty.mid) o+e> ($env.TMPDIR | path join expected); 0 } catch { 0 })
let ec_rust = (try { ^$env.RUST ($env.TMPDIR | path join empty.mid) o+e> ($env.TMPDIR | path join actual); 0 } catch { 0 })
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
$"exit=($ec_ref)\n" | save --raw --append ($env.TMPDIR | path join expected)
$"exit=($ec_rust)\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "pw-mididump empty MTrk (size=0, no events)"
