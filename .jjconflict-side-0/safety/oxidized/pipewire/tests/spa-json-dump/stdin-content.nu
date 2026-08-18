source ../helpers.nu

# Bash used a here-string (<<<) which appends a trailing newline.
let input = "{\"foo\":1,\"bar\":2}\n"
try { $input | ^$env.REF - o+e> ($env.TMPDIR | path join expected) }
try { $input | ^$env.RUST - o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump - <<<'{...}' (stdin parses non-empty input)"
