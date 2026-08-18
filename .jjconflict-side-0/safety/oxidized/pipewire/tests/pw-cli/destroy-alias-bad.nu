source ../helpers.nu

try { ^$env.REF d 99 garbage o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST d 99 garbage o+e> ($env.TMPDIR | path join actual) }
compare "pw-cli d 99 garbage (alias-aware unknown global)"
