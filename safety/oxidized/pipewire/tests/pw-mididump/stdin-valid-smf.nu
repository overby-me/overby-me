source ../helpers.nu

# The bash original used `|| true`, so $? is always 0.
let smf = 0x[4d546864 00000006 00000001 0060 4d54726b 00000004 00 ff2f00]
try { $smf | ^$env.REF - o+e> ($env.TMPDIR | path join expected) }
let e_ref = 0
try { $smf | ^$env.RUST - o+e> ($env.TMPDIR | path join actual) }
let e_rust = 0
$"exit=($e_ref)\n" | save -a --raw ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-mididump - <minimal-SMF (read SMF from stdin)"
