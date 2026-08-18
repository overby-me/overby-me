source ../helpers.nu

hide-env --ignore-errors PIPEWIRE_REMOTE
hide-env --ignore-errors PIPEWIRE_CORE
try { ^$env.REF -r /nonexistent o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -r /nonexistent o+e> ($env.TMPDIR | path join actual) }
compare "pw-metadata/connect-fail"
