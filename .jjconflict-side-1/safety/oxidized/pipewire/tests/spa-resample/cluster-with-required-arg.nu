source ../helpers.nu

try { ^$env.REF -vc 0 - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -vc 0 - - o+e> ($env.TMPDIR | path join actual) }
"exit=0\n" | save --raw --append ($env.TMPDIR | path join expected)
"exit=0\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "spa-resample -vc 0 (cluster with required-arg flag last)"
