source ../helpers.nu

try { ^$env.REF cl o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST cl o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli cl (alias usage)"
