source ../helpers.nu

try { ^$env.REF cn o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST cn o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli cn (alias usage)"
