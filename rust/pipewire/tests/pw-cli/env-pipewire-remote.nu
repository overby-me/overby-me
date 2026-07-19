source ../helpers.nu

with-env {PIPEWIRE_REMOTE: "non-existent-socket"} { try { ^$env.REF help o+e> ($env.TMPDIR | path join expected) } }
with-env {PIPEWIRE_REMOTE: "non-existent-socket"} { try { ^$env.RUST help o+e> ($env.TMPDIR | path join actual) } }
compare "pw-cli with PIPEWIRE_REMOTE env (non-existent socket → 'Host is down')"
