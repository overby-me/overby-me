source ../helpers.nu

touch ($env.TMPDIR | path join nonsocket)
with-env { PIPEWIRE_REMOTE: ($env.TMPDIR | path join nonsocket) } {
    try { ^$env.REF quit o+e> ($env.TMPDIR | path join expected) }
    try { ^$env.RUST quit o+e> ($env.TMPDIR | path join actual) }
}
compare "pw-cli quit (PIPEWIRE_REMOTE bad → connect error)"
