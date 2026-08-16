source ../helpers.nu

let runtime_dir = $env.TMPDIR | path join nonexistent
let expected = $env.TMPDIR | path join expected
let actual = $env.TMPDIR | path join actual
with-env {PIPEWIRE_RUNTIME_DIR: $runtime_dir} {
    try { ^$env.REF -p /etc/passwd o+e> $expected }
    try { ^$env.RUST -p /etc/passwd o+e> $actual }
}
# Normalize the sandbox path to a stable token before comparing.
^sed -i $"s|($env.TMPDIR)|TMPDIR|g" $expected $actual
# bash recorded `$?` after `|| true`, which is always 0.
"exit=0\n" | save -a --raw $expected
"exit=0\n" | save -a --raw $actual
compare "pw-cat with PIPEWIRE_RUNTIME_DIR=nonexistent (connect-fail)"
