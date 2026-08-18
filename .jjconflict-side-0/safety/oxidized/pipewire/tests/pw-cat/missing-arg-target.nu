source ../helpers.nu

try { ^$env.REF --target o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --target o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat/missing-arg-target"
