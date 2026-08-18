source ../helpers.nu

try { "0x10" | ^$env.REF - o+e> ($env.TMPDIR | path join expected) }
try { "0x10" | ^$env.RUST - o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump - <0x10> (bare number -> empty object)"
