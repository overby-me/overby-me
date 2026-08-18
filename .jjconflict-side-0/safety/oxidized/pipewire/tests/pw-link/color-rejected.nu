source ../helpers.nu

try { ^$env.REF --color o+e> ($env.TMPDIR | path join expected) }
try { ^$env.RUST --color o+e> ($env.TMPDIR | path join actual) }
compare "pw-link --color (rejected: not a valid option)"
