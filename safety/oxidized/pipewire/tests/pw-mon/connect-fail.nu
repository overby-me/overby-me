source ../helpers.nu

# Without a daemon, pw-mon should print can't connect: Host is down.
$env.XDG_RUNTIME_DIR = ($env.TMPDIR | path join no-runtime)
try { ^$env.REF o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST o+e> ($env.TMPDIR | path join actual) }
compare "pw-mon (no daemon)"
