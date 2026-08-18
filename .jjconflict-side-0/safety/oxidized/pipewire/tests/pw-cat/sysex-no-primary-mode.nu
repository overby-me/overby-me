source ../helpers.nu

try { ^$env.REF -s o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -s o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat -s alone (sub-mode requires -p or -r)"
