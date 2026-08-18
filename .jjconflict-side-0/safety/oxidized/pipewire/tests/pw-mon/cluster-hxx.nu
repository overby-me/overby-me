source ../helpers.nu

try { ^$env.REF -hxx o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -hxx o+e> ($env.TMPDIR | path join actual) }
compare "pw-mon/cluster-hxx"
