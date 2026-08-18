source ../helpers.nu

try { ^$env.REF --rate=0 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --rate=0 o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat --rate=0 (zero rejected)"
