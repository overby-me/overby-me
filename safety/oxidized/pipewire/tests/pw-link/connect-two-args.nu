source ../helpers.nu

try { ^$env.REF port-a port-b o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST port-a port-b o+e> ($env.TMPDIR | path join actual) }
compare "pw-link port-a port-b (connect)"
