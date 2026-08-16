source ../helpers.nu

let sock = $env.TMPDIR | path join nonsocket
touch $sock
with-env { PIPEWIRE_REMOTE: $sock } {
    try { ^$env.REF o+e> ($env.TMPDIR | path join expected) }
}
with-env { PIPEWIRE_REMOTE: $sock } {
    try { ^$env.RUST o+e> ($env.TMPDIR | path join actual) }
}
compare "pw-profiler (Connection refused)"
