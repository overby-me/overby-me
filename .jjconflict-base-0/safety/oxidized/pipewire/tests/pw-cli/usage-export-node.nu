source ../helpers.nu

try { ^$env.REF export-node o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST export-node o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli export-node (usage error)"
