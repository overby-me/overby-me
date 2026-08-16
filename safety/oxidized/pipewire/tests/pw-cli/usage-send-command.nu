source ../helpers.nu

# pw-cli send-command with no args prints `Error: "send-command <usage>"` to stderr.
try { ^$env.REF send-command o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST send-command o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli send-command (usage error)"
