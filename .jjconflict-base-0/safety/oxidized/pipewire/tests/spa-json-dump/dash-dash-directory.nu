source ../helpers.nu

try { ^$env.REF -- /etc o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -- /etc o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump -- /etc (-- terminator + directory)"
