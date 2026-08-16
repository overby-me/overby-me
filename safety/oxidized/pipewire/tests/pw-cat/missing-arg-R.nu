source ../helpers.nu

try { ^$env.REF -R o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -R o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat/missing-arg-R"
