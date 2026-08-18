source ../helpers.nu

try { ^$env.REF -n o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -n o+e> ($env.TMPDIR | path join actual) }
compare "pw-config -n (short form requires arg)"
