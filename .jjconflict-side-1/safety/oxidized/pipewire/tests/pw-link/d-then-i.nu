source ../helpers.nu

try { ^$env.REF -d -i o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -d -i o+e> ($env.TMPDIR | path join actual) }
compare "pw-link -d -i (last mode flag wins)"
