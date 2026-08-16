source ../helpers.nu

try { ^$env.REF e o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST e o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli e (alias usage)"
