source ../helpers.nu

try { ^$env.REF -- /nonexistent_xyz o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -- /nonexistent_xyz o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump -- /nonexistent (-- terminator + missing file)"
