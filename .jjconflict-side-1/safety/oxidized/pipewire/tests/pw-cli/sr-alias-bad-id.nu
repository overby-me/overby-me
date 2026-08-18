source ../helpers.nu

# Short alias `sr` for switch-remote: same error message.
try { ^$env.REF "sr 99" o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST "sr 99" o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli sr 99 (alias, same Remote N does not exist error)"
