source ../helpers.nu

try { ^$env.REF -rfoo - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -rfoo - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample -rfoo (non-numeric → 'bad rate foo')"
