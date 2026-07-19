source ../helpers.nu

try { ^$env.REF --remote o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --remote o+e> ($env.TMPDIR | path join actual) }
compare "pw-mon --remote (long form requires arg)"
