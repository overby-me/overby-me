source ../helpers.nu

# spa-monitor with no args prints `usage: ... <plugin.so>`.
try { ^$env.REF o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST o+e> ($env.TMPDIR | path join actual) }
compare "spa-monitor/usage"
