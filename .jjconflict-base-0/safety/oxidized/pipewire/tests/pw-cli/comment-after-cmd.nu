source ../helpers.nu

# `pw-cli 'help # trailing'`: C truncates at '#', leaving "help " to
# be parsed. We run the `help` command and ignore the comment.
try { ^$env.REF "help # trailing comment" o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST "help # trailing comment" o+e> ($env.TMPDIR | path join actual) }
# bash read $? after `|| true`, so the recorded exit code is always 0.
"exit=0\n" | save -a --raw ($env.TMPDIR | path join expected)
"exit=0\n" | save -a --raw ($env.TMPDIR | path join actual)
compare "pw-cli 'help # comment' (truncate at # then run help)"
