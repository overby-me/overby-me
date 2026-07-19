source ../helpers.nu

try { ^$env.REF -i -d o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -i -d o+e> ($env.TMPDIR | path join actual) }
compare "pw-link -i -d (last mode flag wins)"
