source ../helpers.nu

# Both tools exit 255 on dlopen failure. Wrap in `try` so a non-zero
# exit doesn't abort the fixture before we reach compare().
try { ^$env.REF /nonexistent.so o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST /nonexistent.so o+e> ($env.TMPDIR | path join actual) }
compare "spa-inspect/bad-plugin"
