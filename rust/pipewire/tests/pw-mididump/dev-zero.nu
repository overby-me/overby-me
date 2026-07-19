source ../helpers.nu

# /dev/zero reads forever; pw-mididump must bound the read and report
# "Invalid argument" (MThd magic mismatch).
let ec_ref = (try { ^$env.REF /dev/zero o+e> ($env.TMPDIR | path join expected); 0 } catch { 0 })
let ec_rust = (try { ^$env.RUST /dev/zero o+e> ($env.TMPDIR | path join actual); 0 } catch { 0 })
$"exit=($ec_ref)\n" | save --raw --append ($env.TMPDIR | path join expected)
$"exit=($ec_rust)\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "pw-mididump /dev/zero (unbounded source → Invalid argument)"
