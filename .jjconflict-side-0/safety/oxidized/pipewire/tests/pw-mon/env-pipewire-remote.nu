source ../helpers.nu

with-env {PIPEWIRE_REMOTE: "non-existent-socket"} {
    try { ^$env.REF o+e> ($env.TMPDIR | path join expected) }
    try { ^$env.RUST o+e> ($env.TMPDIR | path join actual) }
}
compare "pw-mon with PIPEWIRE_REMOTE env (non-existent socket → can't connect)"
