source ../helpers.nu

# `pw-cli ""` joins positionals into "" and parse() returns true
# (no-op success) on empty/whitespace input. Exits silently with code 0,
# unlike `pw-cli` (no args) which connects and starts the REPL.
try { ^$env.REF "" o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST "" o+e> ($env.TMPDIR | path join actual) }
# bash read $? after `|| true`, so the recorded exit code is always 0.
"exit=0\n" | save -a --raw ($env.TMPDIR | path join expected)
"exit=0\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-cli '' (empty command is silent no-op, not help dump)"
