source ../helpers.nu

try { ^$env.REF -o o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -o o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat -o alone (sub-mode requires -p or -r)"
