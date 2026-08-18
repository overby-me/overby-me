source ../helpers.nu

# PIPEWIRE_REMOTE pointing to non-socket file: any command attempts
# connect on startup, errors with "Connection refused".
touch ($env.TMPDIR | path join nonsocket)
with-env { PIPEWIRE_REMOTE: ($env.TMPDIR | path join nonsocket) } {
    try { ^$env.REF help o+e> ($env.TMPDIR | path join expected) }
    try { ^$env.RUST help o+e> ($env.TMPDIR | path join actual) }
}
compare "pw-cli help with PIPEWIRE_REMOTE=non-socket"
