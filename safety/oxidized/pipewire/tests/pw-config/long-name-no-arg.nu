source ../helpers.nu

try { ^$env.REF --name o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --name o+e> ($env.TMPDIR | path join actual) }
compare "pw-config --name (long form requires arg)"
