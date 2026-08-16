source ../helpers.nu

try { ^$env.REF -I o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -I o+e> ($env.TMPDIR | path join actual) }
compare "pw-link -I (without -i/-o/-l)"
