source ../helpers.nu

try { ^$env.REF -M o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -M o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat/missing-arg-M"
