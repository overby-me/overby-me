source ../helpers.nu

# Test size --common: common symbol sizes folded into bss
try { ^$env.REF --common $env.TESTOBJ o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --common $env.TESTOBJ o+e> ($env.TMPDIR | path join actual) }
compare "size --common"
