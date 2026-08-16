source ../helpers.nu

try { ^$env.REF -p o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -p o+e> ($env.TMPDIR | path join actual) }
compare "spa-acp-tool/missing-arg-p"
