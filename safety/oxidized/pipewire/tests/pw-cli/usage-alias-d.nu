source ../helpers.nu

try { ^$env.REF d o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST d o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli d (alias usage)"
