source ../helpers.nu

try { ^$env.REF um foo bar o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST um foo bar o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli um foo bar (alias-aware)"
