source ../helpers.nu

try { ^$env.REF -Rnon-existent -p /etc/passwd o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -Rnon-existent -p /etc/passwd o+e> ($env.TMPDIR | path join actual) }
"exit=0\n" | save -a --raw ($env.TMPDIR | path join expected)
"exit=0\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-cat -Rnon-existent (attached short value → connect-fail)"
