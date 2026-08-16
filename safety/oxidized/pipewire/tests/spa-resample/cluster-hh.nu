source ../helpers.nu

try { ^$env.REF -hh o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -hh o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample/cluster-hh"
