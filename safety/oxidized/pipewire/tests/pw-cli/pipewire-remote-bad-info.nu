source ../helpers.nu

touch ($env.TMPDIR | path join nonsocket)
with-env { PIPEWIRE_REMOTE: ($env.TMPDIR | path join nonsocket) } {
    try { ^$env.REF i Core o+e> ($env.TMPDIR | path join expected) }
    try { ^$env.RUST i Core o+e> ($env.TMPDIR | path join actual) }
}
compare "pw-cli i Core (PIPEWIRE_REMOTE bad → connect error)"
