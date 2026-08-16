source ../helpers.nu

try { "true" | ^$env.REF - o+e> ($env.TMPDIR | path join expected) }
try { "true" | ^$env.RUST - o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump - <true> (bare bool -> empty object)"
