source ../helpers.nu

try { ^$env.REF -x o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -x o+e> ($env.TMPDIR | path join actual) }
compare "pw-mon/short-bad-flag"
