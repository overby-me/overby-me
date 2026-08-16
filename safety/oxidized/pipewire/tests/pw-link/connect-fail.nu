source ../helpers.nu

# pw-link with no daemon socket: C prints `can't connect: Host is down`
# (no `Error:` wrapper, different from pw-cli) and exits 255.
hide-env --ignore-errors PIPEWIRE_REMOTE
hide-env --ignore-errors PIPEWIRE_CORE
try { ^$env.REF -r /nonexistent -i o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -r /nonexistent -i o+e> ($env.TMPDIR | path join actual) }
compare "pw-link connect-fail"
