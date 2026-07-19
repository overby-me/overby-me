source ../helpers.nu

try { ^$env.REF -P o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -P o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat/missing-arg-P"
