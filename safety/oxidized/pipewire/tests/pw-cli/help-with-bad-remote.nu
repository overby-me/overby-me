source ../helpers.nu

try { ^$env.REF help -rfoo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST help -rfoo o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli help -rfoo (startup connect with bad remote)"
