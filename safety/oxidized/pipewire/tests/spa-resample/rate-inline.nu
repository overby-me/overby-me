source ../helpers.nu

try { ^$env.REF --rate=100 a o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --rate=100 a o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --rate=100 a (inline value)"
