source ../helpers.nu

try { ^$env.REF destroy 99 garbage o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST destroy 99 garbage o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli destroy 99 garbage (unknown global)"
