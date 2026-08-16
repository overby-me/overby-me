source ../helpers.nu

# pw-cli with no daemon socket: the C tool's protocol-native client maps
# the underlying ENOENT to EHOSTDOWN, then parse() wraps the message in
# `Error: "..."`. Unset both PIPEWIRE_REMOTE and PIPEWIRE_CORE env vars
# (the daemon-test wrapper sets them) and explicitly point at a path
# that doesn't exist.
with-env {PIPEWIRE_REMOTE: null, PIPEWIRE_CORE: null} {
    try { ^$env.REF -r /nonexistent ls Core o+e> ($env.TMPDIR | path join expected) }
    try { ^$env.RUST -r /nonexistent ls Core o+e> ($env.TMPDIR | path join actual) }
}
compare "pw-cli connect-fail"
