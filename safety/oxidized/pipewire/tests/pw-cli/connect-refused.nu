source ../helpers.nu

# When PIPEWIRE_REMOTE points to a non-socket file, the connect fails
# with ECONNREFUSED instead of ENOENT: verify the wording.
touch ($env.TMPDIR | path join nonsocket)
with-env {PIPEWIRE_REMOTE: ($env.TMPDIR | path join nonsocket)} { try { ^$env.REF ls o+e> ($env.TMPDIR | path join expected) } }
with-env {PIPEWIRE_REMOTE: ($env.TMPDIR | path join nonsocket)} { try { ^$env.RUST ls o+e> ($env.TMPDIR | path join actual) } }
compare "pw-cli ls (PIPEWIRE_REMOTE=non-socket → Connection refused)"
