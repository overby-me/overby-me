source ../helpers.nu

try { ^$env.REF foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-link foo (connect mode, missing input)"
