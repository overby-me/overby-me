source ../helpers.nu

"{ a = 1 b = two }\n" | save -f --raw ($env.TMPDIR | path join in.json)
open --raw ($env.TMPDIR | path join in.json) | ^$env.REF - o+e> ($env.TMPDIR | path join expected)
open --raw ($env.TMPDIR | path join in.json) | ^$env.RUST - o+e> ($env.TMPDIR | path join actual)
compare "spa-json-dump - (stdin marker)"
