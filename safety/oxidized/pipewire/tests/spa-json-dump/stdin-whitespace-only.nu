source ../helpers.nu

let input = "   \n  \t \n"
try { $input | ^$env.REF - o+e> ($env.TMPDIR | path join expected) }
try { $input | ^$env.RUST - o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump - <whitespace-only> ('not a valid file')"
