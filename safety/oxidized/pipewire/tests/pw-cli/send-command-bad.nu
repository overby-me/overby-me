source ../helpers.nu

try { ^$env.REF send-command 0 0 0 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST send-command 0 0 0 o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli send-command 0 0 0 (no daemon)"
