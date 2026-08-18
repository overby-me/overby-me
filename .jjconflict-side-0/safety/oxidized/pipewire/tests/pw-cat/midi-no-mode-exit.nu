source ../helpers.nu

try { ^$env.REF -m o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -m o+e> ($env.TMPDIR | path join actual) }
"exit=0\n" | save -a --raw ($env.TMPDIR | path join expected)
"exit=0\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-cat -m alone (no primary mode → exit 1)"
