source ../helpers.nu

try { ^$env.REF get-permissions o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST get-permissions o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli get-permissions (usage error)"
