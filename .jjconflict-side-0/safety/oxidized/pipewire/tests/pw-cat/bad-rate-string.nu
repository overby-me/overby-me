source ../helpers.nu

try { ^$env.REF --rate=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --rate=foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat --rate=foo (atoi → 0 → bad rate 0)"
