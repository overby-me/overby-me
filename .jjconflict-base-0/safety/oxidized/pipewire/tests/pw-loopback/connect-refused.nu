source ../helpers.nu

touch ($env.TMPDIR | path join nonsocket)
with-env {PIPEWIRE_REMOTE: ($env.TMPDIR | path join nonsocket)} {
    try { ^$env.REF o+e> ($env.TMPDIR | path join expected) }
    try { ^$env.RUST o+e> ($env.TMPDIR | path join actual) }
}
compare "pw-loopback (Connection refused)"
