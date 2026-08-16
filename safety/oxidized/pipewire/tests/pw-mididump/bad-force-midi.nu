source ../helpers.nu

try { ^$env.REF -M invalid o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -M invalid o+e> ($env.TMPDIR | path join actual) }
compare "pw-mididump/bad-force-midi"
