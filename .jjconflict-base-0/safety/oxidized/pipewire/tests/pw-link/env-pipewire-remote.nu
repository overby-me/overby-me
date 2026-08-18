source ../helpers.nu

with-env {PIPEWIRE_REMOTE: "non-existent-socket"} {
    try { ^$env.REF a b o+e> ($env.TMPDIR | path join expected) }
    try { ^$env.RUST a b o+e> ($env.TMPDIR | path join actual) }
}
compare "pw-link with PIPEWIRE_REMOTE env (non-existent socket -> can't connect)"
