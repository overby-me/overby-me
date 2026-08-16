source ../helpers.nu

try { ^$env.REF -c o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -c o+e> ($env.TMPDIR | path join actual) }
compare "spa-acp-tool/missing-arg-c"
