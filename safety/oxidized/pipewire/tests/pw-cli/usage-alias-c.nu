source ../helpers.nu

try { ^$env.REF c o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST c o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli c (alias usage)"
