source ../helpers.nu

try { ^$env.REF gp o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST gp o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli gp (alias usage)"
