source ../helpers.nu

try { ^$env.REF --channels=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --channels=foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-cat --channels=foo (atoi → 0 → bad channels 0)"
