source ../helpers.nu

try { ^$env.REF --color=foo o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --color=foo o+e> ($env.TMPDIR | path join actual) }
compare "pw-mon --color=foo (invalid color)"
