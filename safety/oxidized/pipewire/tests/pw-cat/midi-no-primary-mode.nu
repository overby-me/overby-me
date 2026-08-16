source ../helpers.nu

try { ^$env.REF -m o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -m o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat -m alone (sub-mode requires -p or -r)"
