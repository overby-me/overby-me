source ../helpers.nu

try { ^$env.REF -Vxx o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -Vxx o+e> ($env.TMPDIR | path join actual) }
compare "pw-dot/cluster-Vxx"
