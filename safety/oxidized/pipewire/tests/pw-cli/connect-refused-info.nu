source ../helpers.nu

touch ($env.TMPDIR | path join nonsocket)
with-env {PIPEWIRE_REMOTE: ($env.TMPDIR | path join nonsocket)} { try { ^$env.REF i 0 o+e> ($env.TMPDIR | path join expected) } }
with-env {PIPEWIRE_REMOTE: ($env.TMPDIR | path join nonsocket)} { try { ^$env.RUST i 0 o+e> ($env.TMPDIR | path join actual) } }
compare "pw-cli i 0 (Connection refused)"
