source ../helpers.nu

# spa-json-dump on a non-existent file: matches C's
# `error opening file 'X': <reason>` and exit 1.
try { ^$env.REF /nonexistent.json o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST /nonexistent.json o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump/bad-file"
