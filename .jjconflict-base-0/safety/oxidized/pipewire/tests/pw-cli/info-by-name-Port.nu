source ../helpers.nu

try { ^$env.REF info Port o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST info Port o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli info Port (by-name lookup, rich daemon)"
