source ../helpers.nu

$env.XDG_RUNTIME_DIR = ($env.TMPDIR | path join no-runtime)
hide-env -i PIPEWIRE_REMOTE
try { ^$env.REF o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST o+e> ($env.TMPDIR | path join actual) }
compare "pw-mididump (no daemon → can't connect)"
