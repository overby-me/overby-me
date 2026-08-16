source ../helpers.nu

try { ^$env.REF -Mump /tmp/nonexistent o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -Mump /tmp/nonexistent o+e> ($env.TMPDIR | path join actual) }
compare "pw-mididump -Mump <file> (attached value)"
