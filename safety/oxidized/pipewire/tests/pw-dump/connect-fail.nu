source ../helpers.nu

hide-env -i PIPEWIRE_REMOTE
hide-env -i PIPEWIRE_CORE
try { ^$env.REF -r /nonexistent o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -r /nonexistent o+e> ($env.TMPDIR | path join actual) }
compare "pw-dump/connect-fail"
