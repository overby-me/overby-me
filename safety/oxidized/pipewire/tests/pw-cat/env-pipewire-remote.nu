source ../helpers.nu

with-env {PIPEWIRE_REMOTE: "non-existent-socket"} {
    try { ^$env.REF -p /etc/passwd o+e> ($env.TMPDIR | path join expected) }
    try { ^$env.RUST -p /etc/passwd o+e> ($env.TMPDIR | path join actual) }
}
compare "pw-cat -p with PIPEWIRE_REMOTE env (non-existent socket → connect failed)"
