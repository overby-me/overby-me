source ../helpers.nu

let e_ref = try { ^$env.REF --remote o+e> ($env.TMPDIR | path join expected); 0 } catch { |e| $e.exit_code }
let e_rust = try { ^$env.RUST --remote o+e> ($env.TMPDIR | path join actual); 0 } catch { |e| $e.exit_code }
$"exit=($e_ref)\n" | save -a --raw ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-metadata --remote (long form requires arg)"
