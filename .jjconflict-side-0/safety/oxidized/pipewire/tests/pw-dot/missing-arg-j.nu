source ../helpers.nu

try { ^$env.REF -j o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -j o+e> ($env.TMPDIR | path join actual) }
compare "pw-dot/missing-arg-j"
