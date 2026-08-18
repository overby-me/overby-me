source ../helpers.nu

try { ^$env.REF -o o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -o o+e> ($env.TMPDIR | path join actual) }
compare "pw-dot/missing-arg-o"
