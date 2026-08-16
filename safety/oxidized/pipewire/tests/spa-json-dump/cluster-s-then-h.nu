source ../helpers.nu

try { ^$env.REF -sh o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST -sh o+e> ($env.TMPDIR | path join actual) }
compare "spa-json-dump -sh (-s sets simple, -h prints help)"
