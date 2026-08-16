source ../helpers.nu

try { ^$env.REF o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST o+e> ($env.TMPDIR | path join actual) }
# bash captured $? after `|| true`, which is always 0; mirror that exactly.
"exit=0\n" | save --raw --append ($env.TMPDIR | path join expected)
"exit=0\n" | save --raw --append ($env.TMPDIR | path join actual)
compare "spa-inspect (no args) exits 255"
