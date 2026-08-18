source ../helpers.nu

try { ^$env.REF -r o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -r o+e> ($env.TMPDIR | path join actual) }
compare "pw-link/missing-arg-r"
