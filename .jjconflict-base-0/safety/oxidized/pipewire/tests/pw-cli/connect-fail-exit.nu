source ../helpers.nu

with-env {PIPEWIRE_REMOTE: "non-existent"} { try { ^$env.REF ls o+e> ($env.TMPDIR | path join expected) } }
with-env {PIPEWIRE_REMOTE: "non-existent"} { try { ^$env.RUST ls o+e> ($env.TMPDIR | path join actual) } }
# bash read $? after `|| true`, so the recorded exit code is always 0.
"exit=0\n" | save -a --raw ($env.TMPDIR | path join expected)
"exit=0\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-cli ls connect-fail exits 255"
