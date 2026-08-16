source ../helpers.nu

touch ($env.TMPDIR | path join nonsocket)
with-env {PIPEWIRE_REMOTE: ($env.TMPDIR | path join nonsocket)} { try { ^$env.REF help o+e> ($env.TMPDIR | path join expected) } }
with-env {PIPEWIRE_REMOTE: ($env.TMPDIR | path join nonsocket)} { try { ^$env.RUST help o+e> ($env.TMPDIR | path join actual) } }
compare "pw-cli help (Connection refused via PIPEWIRE_REMOTE)"
