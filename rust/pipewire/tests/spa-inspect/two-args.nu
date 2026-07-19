source ../helpers.nu

try { ^$env.REF /foo.so /bar.so o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST /foo.so /bar.so o+e> ($env.TMPDIR | path join actual) }
compare "spa-inspect/two-args"
