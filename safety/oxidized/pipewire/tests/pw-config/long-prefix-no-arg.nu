source ../helpers.nu

try { ^$env.REF --prefix o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --prefix o+e> ($env.TMPDIR | path join actual) }
compare "pw-config --prefix (long form requires arg)"
