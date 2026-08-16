source ../helpers.nu

with-env {PIPEWIRE_REMOTE: "non-existent-socket"} {
    try { ^$env.REF -l o+e> ($env.TMPDIR | path join expected) }
    try { ^$env.RUST -l o+e> ($env.TMPDIR | path join actual) }
}
compare "pw-link -l with PIPEWIRE_REMOTE env (list mode -> can't connect)"
