source ../helpers.nu

try { ^$env.REF -q o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -q o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat/missing-arg-q"
