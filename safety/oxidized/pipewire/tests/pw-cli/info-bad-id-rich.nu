source ../helpers.nu

try { ^$env.REF info 99999 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST info 99999 o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli info 99999 (rich daemon)"
