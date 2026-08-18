source ../helpers.nu

# The bash original used `|| true`, so $? is always 0; both exit lines
# below are always exit=0.
try { ^$env.REF o+e> ($env.TMPDIR | path join expected) }
let e_ref = 0
try { ^$env.RUST o+e> ($env.TMPDIR | path join actual) }
let e_rust = 0
$"exit=($e_ref)\n" | save -a --raw ($env.TMPDIR | path join expected)
$"exit=($e_rust)\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-mididump no args (sandbox: connect-fail)"
