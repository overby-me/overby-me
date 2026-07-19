source ../helpers.nu

try { ^$env.REF -Vh o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -Vh o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat/cluster-Vh"
