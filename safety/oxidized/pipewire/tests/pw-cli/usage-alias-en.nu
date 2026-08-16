source ../helpers.nu

try { ^$env.REF en o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST en o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli en (alias usage)"
