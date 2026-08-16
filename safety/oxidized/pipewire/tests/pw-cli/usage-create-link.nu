source ../helpers.nu

try { ^$env.REF create-link o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST create-link o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli create-link (usage error)"
