source ../helpers.nu

try { ^$env.REF um o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST um o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli um (alias usage)"
