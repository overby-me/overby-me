source ../helpers.nu

let nonsocket = $env.TMPDIR | path join nonsocket
touch $nonsocket
with-env {PIPEWIRE_REMOTE: $nonsocket} { try { ^$env.REF o+e> ($env.TMPDIR | path join expected) } }
with-env {PIPEWIRE_REMOTE: $nonsocket} { try { ^$env.RUST o+e> ($env.TMPDIR | path join actual) } }
compare "pw-dump (Connection refused)"
