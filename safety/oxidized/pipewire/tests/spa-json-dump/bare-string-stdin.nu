source ../helpers.nu

try { '"hello"' | ^$env.REF - o+e> ($env.TMPDIR | path join expected) }
try { '"hello"' | ^$env.RUST - o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump - <\"hello\"> (bare string -> empty object)"
