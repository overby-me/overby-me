source ../helpers.nu

try { ^$env.REF -hV o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -hV o+e> ($env.TMPDIR | path join actual) }
compare "pw-top/cluster-hV"
