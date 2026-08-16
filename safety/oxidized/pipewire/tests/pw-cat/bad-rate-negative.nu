source ../helpers.nu

try { ^$env.REF --rate=-5 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --rate=-5 o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat --rate=-5 (negative rejected)"
