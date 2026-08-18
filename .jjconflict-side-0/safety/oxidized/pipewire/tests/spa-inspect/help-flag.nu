source ../helpers.nu

try { ^$env.REF -h o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -h o+e> ($env.TMPDIR | path join actual) }
compare "spa-inspect/help-flag"
