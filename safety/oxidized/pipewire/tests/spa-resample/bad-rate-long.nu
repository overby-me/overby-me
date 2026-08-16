source ../helpers.nu

try { ^$env.REF --rate=foo - - o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --rate=foo - - o+e> ($env.TMPDIR | path join actual) }
compare "spa-resample --rate=foo (non-numeric long form)"
