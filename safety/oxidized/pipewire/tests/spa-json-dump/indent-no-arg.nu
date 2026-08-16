source ../helpers.nu

try { ^$env.REF --indent o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --indent o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump --indent (long form requires arg)"
