source ../helpers.nu

try { ^$env.REF -hv o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -hv o+e> ($env.TMPDIR | path join actual) }
compare "spa-acp-tool -hv (cluster)"
