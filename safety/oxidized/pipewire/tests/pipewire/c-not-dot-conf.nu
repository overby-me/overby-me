source ../helpers.nu

try { ^$env.REF -c foo o+e> ($env.TMPDIR | path join expected) }
let e_ref = $env.LAST_EXIT_CODE
try { ^$env.RUST -c foo o+e> ($env.TMPDIR | path join actual) }
let e_rust = $env.LAST_EXIT_CODE
# Normalize timestamps in C output.
^sed -i -E 's/\[[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{6}\]/[TIME]/g' ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
$"exit=($e_ref)\n" | save -a --raw ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pipewire -c foo (no .conf suffix → exit 234)"
