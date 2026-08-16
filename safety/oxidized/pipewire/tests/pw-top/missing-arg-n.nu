source ../helpers.nu

try { ^$env.REF -n o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -n o+e> ($env.TMPDIR | path join actual) }
compare "pw-top/missing-arg-n"
