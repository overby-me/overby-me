source ../helpers.nu

let expected = ($env.TMPDIR | path join expected)
let actual = ($env.TMPDIR | path join actual)
# `cmd || true` swallows the failure, so `$?` is always 0 here (as in bash).
let e_ref = (try { ^$env.REF --bogus o+e> $expected; 0 } catch { 0 })
let e_rust = (try { ^$env.RUST --bogus o+e> $actual; 0 } catch { 0 })
$"exit=($e_ref)\n" | save --append --raw $expected
$"exit=($e_rust)\n" | save --append --raw $actual
compare "spa-json-dump --bogus exits 255"
