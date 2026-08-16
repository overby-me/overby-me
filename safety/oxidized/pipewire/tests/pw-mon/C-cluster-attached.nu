source ../helpers.nu

try { ^$env.REF -Cnever o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -Cnever o+e> ($env.TMPDIR | path join actual) }
compare "pw-mon -Cnever (-C is no-arg, -n unknown)"
