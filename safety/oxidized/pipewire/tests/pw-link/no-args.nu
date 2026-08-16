source ../helpers.nu

try { ^$env.REF o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST o+e> ($env.TMPDIR | path join actual) }
# bash captured $? after `|| true`, which is always 0, so reproduce exit=0.
"exit=0\n" | save -a --raw ($env.TMPDIR | path join expected)
"exit=0\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-link no args (sandbox: missing port names)"
