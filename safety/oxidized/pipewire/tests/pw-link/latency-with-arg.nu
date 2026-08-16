source ../helpers.nu

try { ^$env.REF --latency=100 o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --latency=100 o+e> ($env.TMPDIR | path join actual) }
compare "pw-link --latency=100 (no-arg flag rejects value)"
