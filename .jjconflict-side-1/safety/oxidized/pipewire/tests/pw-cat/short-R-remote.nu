source ../helpers.nu

try { ^$env.REF -R non-existent -p /etc/passwd o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -R non-existent -p /etc/passwd o+e> ($env.TMPDIR | path join actual) }
"exit=0\n" | save -a --raw ($env.TMPDIR | path join expected)
"exit=0\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-cat -R non-existent (explicit remote → connect-fail)"
