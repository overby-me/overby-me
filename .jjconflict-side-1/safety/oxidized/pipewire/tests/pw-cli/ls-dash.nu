source ../helpers.nu

try { ^$env.REF ls "-" o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST ls "-" o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli ls - (lone dash positional)"
