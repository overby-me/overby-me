source ../helpers.nu

try { ^$env.REF --bad-flag o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --bad-flag o+e> ($env.TMPDIR | path join actual) }
compare "pw-loopback/bad-flag"
