source ../helpers.nu

try { ^$env.REF - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST - o+e> ($env.TMPDIR | path join actual) }
compare "pw-link - (lone dash positional)"
