source ../helpers.nu

try { ^$env.REF load-module foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST load-module foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli load-module foo (could not load)"
