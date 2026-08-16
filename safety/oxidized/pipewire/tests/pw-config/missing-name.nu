source ../helpers.nu

try { ^$env.REF -n nonexistent.conf paths o+e> ($env.TMPDIR | path join expected) }
let e_ref = $env.LAST_EXIT_CODE
try { ^$env.RUST -n nonexistent.conf paths o+e> ($env.TMPDIR | path join actual) }
let e_rust = $env.LAST_EXIT_CODE
^sed -i -E 's/\[[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{6}\]/[TIME]/g; s/0x[0-9a-fA-F]+/0xPTR/g' ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
$"exit=($e_ref)\n" | save --raw --append ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "pw-config -n nonexistent.conf (no prefix → 2-line log)"
