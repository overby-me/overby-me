source ../helpers.nu

# The bash original used `|| true`, which makes $? always 0, so both exit
# lines below are always exit=0.
try { ^$env.REF missing-file o+e> ($env.TMPDIR | path join expected) }
let e_ref = 0
try { ^$env.RUST missing-file o+e> ($env.TMPDIR | path join actual) }
let e_rust = 0
^sed -i "s|0x[0-9a-fA-F]*|0xPTR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
$"exit=($e_ref)\n" | save --append --raw ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save --append --raw ($env.TMPDIR | path join actual)
compare "pw-encplay missing-file (avformat: No such file)"
