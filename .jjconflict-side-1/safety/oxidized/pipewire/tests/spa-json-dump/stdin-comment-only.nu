source ../helpers.nu

let input = "# this is a comment\n# and another\n"
try { $input | ^$env.REF - o+e> ($env.TMPDIR | path join expected) }
try { $input | ^$env.RUST - o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump - <comment-only> ('not a valid file')"
