source ../helpers.nu

try { ^$env.REF sp o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST sp o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli sp (alias usage)"
