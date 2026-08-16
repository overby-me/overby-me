source ../helpers.nu

with-env {PIPEWIRE_RUNTIME_DIR: ($env.TMPDIR | path join nonexistent)} { try { ^$env.REF o+e> ($env.TMPDIR | path join expected) } }
with-env {PIPEWIRE_RUNTIME_DIR: ($env.TMPDIR | path join nonexistent)} { try { ^$env.RUST o+e> ($env.TMPDIR | path join actual) } }
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" ($env.TMPDIR | path join expected) ($env.TMPDIR | path join actual)
# bash read $? after `|| true`, so the recorded exit code is always 0.
"exit=0\n" | save -a --raw ($env.TMPDIR | path join expected)
"exit=0\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-cli with PIPEWIRE_RUNTIME_DIR (non-existent → exit 255)"
