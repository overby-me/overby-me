source ../helpers.nu

try { ^$env.REF -hp o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -hp o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat/cluster-hp"
