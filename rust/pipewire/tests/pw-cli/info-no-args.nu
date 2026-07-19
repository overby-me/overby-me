source ../helpers.nu

try { ^$env.REF info o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST info o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli info (no args)"
