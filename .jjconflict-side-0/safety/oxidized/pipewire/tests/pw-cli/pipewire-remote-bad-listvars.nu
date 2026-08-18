source ../helpers.nu

touch ($env.TMPDIR | path join nonsocket)
with-env { PIPEWIRE_REMOTE: ($env.TMPDIR | path join nonsocket) } {
    try { ^$env.REF list-vars o+e> ($env.TMPDIR | path join expected) }
    try { ^$env.RUST list-vars o+e> ($env.TMPDIR | path join actual) }
}
compare "pw-cli list-vars (PIPEWIRE_REMOTE bad → connect error)"
